//! Contract tests for the product workflow facade.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use ironclaw_auth::{AuthFlowId, CredentialAccountId};
use ironclaw_conversations::{
    ConversationBindingService as ConversationBindingPort, InMemoryConversationServices,
};
use ironclaw_host_api::{AgentId, ApprovalRequestId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_product_adapters::{
    AdapterInstallationId, ApprovalDecision, ApprovalResolutionPayload, AuthRequirement,
    AuthResolutionPayload, AuthResolutionResult, ExternalActorRef, ExternalConversationRef,
    ExternalEventId, InboundCommandPayload, LinkedThreadActionPayload, ParsedProductInbound,
    ProductAdapterError, ProductAdapterId, ProductControlActionPayload, ProductInboundAck,
    ProductInboundEnvelope, ProductInboundPayload, ProductProjectionReadInput,
    ProductProjectionSubject, ProductProjectionSubscribeInput, ProductRejection,
    ProductRejectionDisposition, ProductRejectionKind, ProductTriggerReason, ProductWorkflow,
    ProductWorkflowRejectionKind, ProjectionCursor, ProjectionReadPayload,
    ProjectionSubscriptionPayload, ProtocolAuthEvidence, ScopedApprovalResolutionPayload,
    TrustedInboundContext, UserMessagePayload,
};
use ironclaw_product_workflow::{
    ActionDispatchKind, ActionFingerprintKey, ApprovalInteractionDecision,
    ApprovalInteractionScope, ApprovalInteractionService, AuthInteractionDecision,
    AuthInteractionScope, AuthInteractionService, AuthInteractionStatus, AuthRequestRef,
    BeforeInboundPolicy, BeforeInboundPolicyOutcome, BeforeInboundPolicyRequest,
    ConversationBindingService, DefaultInboundTurnService, DefaultProductWorkflow,
    FakeBeforeInboundPolicy, FakeConversationBindingService, FakeIdempotencyLedger,
    FakeInboundTurnService, IdempotencyDecision, IdempotencyLedger, InMemoryIdempotencyLedger,
    InboundTurnOutcome, InboundTurnService, InboundUserMessageDispatch, LinkedThreadActionId,
    ListPendingApprovalsRequest, ListPendingApprovalsResponse, ListPendingAuthInteractionsRequest,
    ListPendingAuthInteractionsResponse, PendingApprovalInteractionView,
    PendingAuthInteractionView, ProductActorUserResolutionRequest, ProductActorUserResolver,
    ProductCommandName, ProductConversationBindingService, ProductConversationRouteKey,
    ProductConversationSubjectRouteResolutionRequest, ProductConversationSubjectRouteResolver,
    ProductInstallationKey, ProductInstallationScope, ProductWorkflowError,
    ResolveApprovalInteractionRequest, ResolveApprovalInteractionResponse,
    ResolveAuthInteractionRequest, ResolveAuthInteractionResponse, ResolveBindingRequest,
    ResolvedBinding, SourceBindingKey, StaticProductInstallationResolver, approval_gate_ref,
};
use ironclaw_threads::InMemorySessionThreadService;
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, EventCursor, GateRef,
    GetRunStateRequest, LoopGateRef, ResumeTurnRequest, ResumeTurnResponse, RunProfileId,
    RunProfileVersion, SubmitTurnRequest, SubmitTurnResponse, ThreadBusy, TurnActor,
    TurnCoordinator, TurnError, TurnId, TurnRunId, TurnRunState, TurnScope, TurnStatus,
};

fn sample_envelope(event_suffix: &str) -> ProductInboundEnvelope {
    sample_envelope_with_payload(
        event_suffix,
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello", vec![], ProductTriggerReason::DirectChat)
                .expect("valid"),
        ),
    )
}

fn sample_noop_envelope(event_suffix: &str) -> ProductInboundEnvelope {
    sample_envelope_with_payload(event_suffix, ProductInboundPayload::NoOp)
}

fn sample_envelope_with_payload(
    event_suffix: &str,
    payload: ProductInboundPayload,
) -> ProductInboundEnvelope {
    sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("valid"),
        AdapterInstallationId::new("install_alpha").expect("valid"),
        ExternalEventId::new(format!("evt:{event_suffix}")).expect("valid"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("valid"),
        ExternalConversationRef::new(None, "conv1", None, None).expect("valid"),
        payload,
    )
}

fn sample_envelope_with_context(
    adapter_id: ProductAdapterId,
    installation_id: AdapterInstallationId,
    external_event_id: ExternalEventId,
    external_actor_ref: ExternalActorRef,
    external_conversation_ref: ExternalConversationRef,
    payload: ProductInboundPayload,
) -> ProductInboundEnvelope {
    let evidence = ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Secret".into(),
        },
        installation_id.as_str(),
    );
    let context = TrustedInboundContext::from_verified_evidence(
        adapter_id,
        installation_id,
        Utc::now(),
        &evidence,
    )
    .expect("verified");

    let parsed = ParsedProductInbound::new(
        external_event_id,
        external_actor_ref,
        external_conversation_ref,
        payload,
    )
    .expect("parsed");

    ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope")
}

#[derive(Default)]
struct RecordingTurnCoordinator {
    submissions: Mutex<Vec<SubmitTurnRequest>>,
    busy_once: Mutex<Option<TurnRunId>>,
}

impl RecordingTurnCoordinator {
    fn submissions(&self) -> Vec<SubmitTurnRequest> {
        self.submissions.lock().expect("lock").clone()
    }

    fn force_thread_busy_once(&self, active_run_id: TurnRunId) {
        *self.busy_once.lock().expect("lock") = Some(active_run_id);
    }
}

#[async_trait]
impl TurnCoordinator for RecordingTurnCoordinator {
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        Ok(TurnRunId::new())
    }

    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        if let Some(active_run_id) = self.busy_once.lock().expect("lock").take() {
            return Err(TurnError::ThreadBusy(ThreadBusy {
                active_run_id,
                status: TurnStatus::Running,
                event_cursor: EventCursor::default(),
            }));
        }
        let response = SubmitTurnResponse::Accepted {
            turn_id: TurnId::new(),
            run_id: TurnRunId::new(),
            status: TurnStatus::Queued,
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            event_cursor: EventCursor::default(),
            accepted_message_ref: request.accepted_message_ref.clone(),
            reply_target_binding_ref: request.reply_target_binding_ref.clone(),
        };
        self.submissions.lock().expect("lock").push(request);
        Ok(response)
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        panic!("resume_turn is not used by product workflow contract tests")
    }

    async fn retry_turn(
        &self,
        _request: ironclaw_turns::RetryTurnRequest,
    ) -> Result<ironclaw_turns::RetryTurnResponse, TurnError> {
        panic!("retry_turn is not used by product workflow contract tests")
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        panic!("cancel_run is not used by product workflow contract tests")
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        panic!("get_run_state is not used by product workflow contract tests")
    }
}

struct RecordingApprovalInteractionService {
    pending: Vec<(GateRef, TurnRunId)>,
    fallback_run_id: TurnRunId,
    resolutions: Mutex<Vec<ResolveApprovalInteractionRequest>>,
}

impl RecordingApprovalInteractionService {
    fn new(gate_ref: GateRef, run_id: TurnRunId) -> Self {
        Self {
            pending: vec![(gate_ref, run_id)],
            fallback_run_id: run_id,
            resolutions: Mutex::new(Vec::new()),
        }
    }

    fn with_pending(pending: Vec<(GateRef, TurnRunId)>) -> Self {
        let fallback_run_id = pending
            .first()
            .map(|(_, run_id)| *run_id)
            .unwrap_or_default();
        Self {
            pending,
            fallback_run_id,
            resolutions: Mutex::new(Vec::new()),
        }
    }

    fn resolutions(&self) -> Vec<ResolveApprovalInteractionRequest> {
        self.resolutions.lock().expect("lock").clone()
    }
}

#[async_trait]
impl ApprovalInteractionService for RecordingApprovalInteractionService {
    async fn list_pending(
        &self,
        request: ListPendingApprovalsRequest,
    ) -> Result<ListPendingApprovalsResponse, ProductWorkflowError> {
        let scope = ApprovalInteractionScope::from_turn(&request.scope, &request.actor);
        Ok(ListPendingApprovalsResponse {
            approvals: self
                .pending
                .iter()
                .map(|(gate_ref, run_id)| PendingApprovalInteractionView {
                    scope: scope.clone(),
                    run_id: *run_id,
                    gate_ref: gate_ref.clone(),
                    approval_request_id: ApprovalRequestId::new(),
                    summary: "Approval required".to_string(),
                    action: ironclaw_product_workflow::ApprovalInteractionActionView::Other,
                })
                .collect(),
        })
    }

    async fn resolve(
        &self,
        request: ResolveApprovalInteractionRequest,
    ) -> Result<ResolveApprovalInteractionResponse, ProductWorkflowError> {
        let run_id = request.run_id_hint.unwrap_or(self.fallback_run_id);
        self.resolutions.lock().expect("lock").push(request);
        Ok(
            match self
                .resolutions
                .lock()
                .expect("lock")
                .last()
                .expect("recorded")
                .decision
            {
                ApprovalInteractionDecision::ApproveOnce
                | ApprovalInteractionDecision::AlwaysAllow => {
                    ResolveApprovalInteractionResponse::Approved(ResumeTurnResponse {
                        run_id,
                        status: TurnStatus::Queued,
                        event_cursor: EventCursor(21),
                    })
                }
                ApprovalInteractionDecision::Deny => {
                    ResolveApprovalInteractionResponse::Denied(CancelRunResponse {
                        run_id,
                        status: TurnStatus::Cancelled,
                        event_cursor: EventCursor(22),
                        already_terminal: false,
                        actor: None,
                    })
                }
            },
        )
    }
}

struct RecordingAuthInteractionService {
    gate_ref: GateRef,
    run_id: TurnRunId,
    resolutions: Mutex<Vec<ResolveAuthInteractionRequest>>,
}

impl RecordingAuthInteractionService {
    fn new(gate_ref: GateRef, run_id: TurnRunId) -> Self {
        Self {
            gate_ref,
            run_id,
            resolutions: Mutex::new(Vec::new()),
        }
    }

    fn resolutions(&self) -> Vec<ResolveAuthInteractionRequest> {
        self.resolutions.lock().expect("lock").clone()
    }
}

#[async_trait]
impl AuthInteractionService for RecordingAuthInteractionService {
    async fn list_pending(
        &self,
        request: ListPendingAuthInteractionsRequest,
    ) -> Result<ListPendingAuthInteractionsResponse, ProductWorkflowError> {
        let scope = AuthInteractionScope::from_turn(&request.scope, &request.actor);
        Ok(ListPendingAuthInteractionsResponse {
            auth_interactions: vec![PendingAuthInteractionView {
                scope,
                run_id: self.run_id,
                auth_request_ref: self.gate_ref.clone(),
                flow_id: ironclaw_auth::AuthFlowId::new(),
                status: AuthInteractionStatus::AwaitingUser,
                provider: ironclaw_auth::AuthProviderId::new("gmail").expect("provider"),
                summary: "Authentication required".to_string(),
                challenge: None,
                expires_at: Utc::now(),
            }],
        })
    }

    async fn resolve(
        &self,
        request: ResolveAuthInteractionRequest,
    ) -> Result<ResolveAuthInteractionResponse, ProductWorkflowError> {
        let run_id = request.run_id_hint.unwrap_or(self.run_id);
        let decision = request.decision.clone();
        self.resolutions.lock().expect("lock").push(request);
        Ok(match decision {
            AuthInteractionDecision::CredentialProvided { .. }
            | AuthInteractionDecision::CallbackCompleted { .. } => {
                ResolveAuthInteractionResponse::Resumed(ResumeTurnResponse {
                    run_id,
                    status: TurnStatus::Queued,
                    event_cursor: EventCursor(31),
                })
            }
            AuthInteractionDecision::Deny => {
                ResolveAuthInteractionResponse::Canceled(CancelRunResponse {
                    run_id,
                    status: TurnStatus::Cancelled,
                    event_cursor: EventCursor(32),
                    already_terminal: false,
                    actor: None,
                })
            }
        })
    }
}

#[derive(Default)]
struct MissingGateThenRecordingApprovalService {
    resolutions: Mutex<Vec<ResolveApprovalInteractionRequest>>,
}

impl MissingGateThenRecordingApprovalService {
    fn resolutions(&self) -> Vec<ResolveApprovalInteractionRequest> {
        self.resolutions.lock().expect("lock").clone()
    }
}

#[async_trait]
impl ApprovalInteractionService for MissingGateThenRecordingApprovalService {
    async fn list_pending(
        &self,
        _request: ListPendingApprovalsRequest,
    ) -> Result<ListPendingApprovalsResponse, ProductWorkflowError> {
        Ok(ListPendingApprovalsResponse {
            approvals: Vec::new(),
        })
    }

    async fn resolve(
        &self,
        request: ResolveApprovalInteractionRequest,
    ) -> Result<ResolveApprovalInteractionResponse, ProductWorkflowError> {
        let run_id_hint = request.run_id_hint;
        self.resolutions.lock().expect("lock").push(request);
        let Some(run_id) = run_id_hint else {
            return Err(ProductWorkflowError::ApprovalInteractionRejected {
                kind: ironclaw_product_workflow::ApprovalInteractionRejectionKind::MissingGate,
            });
        };
        Ok(ResolveApprovalInteractionResponse::Approved(
            ResumeTurnResponse {
                run_id,
                status: TurnStatus::Queued,
                event_cursor: EventCursor(41),
            },
        ))
    }
}

#[derive(Default)]
struct MissingAuthThenRecordingAuthService {
    resolutions: Mutex<Vec<ResolveAuthInteractionRequest>>,
}

impl MissingAuthThenRecordingAuthService {
    fn resolutions(&self) -> Vec<ResolveAuthInteractionRequest> {
        self.resolutions.lock().expect("lock").clone()
    }
}

#[async_trait]
impl AuthInteractionService for MissingAuthThenRecordingAuthService {
    async fn list_pending(
        &self,
        _request: ListPendingAuthInteractionsRequest,
    ) -> Result<ListPendingAuthInteractionsResponse, ProductWorkflowError> {
        Ok(ListPendingAuthInteractionsResponse {
            auth_interactions: Vec::new(),
        })
    }

    async fn resolve(
        &self,
        request: ResolveAuthInteractionRequest,
    ) -> Result<ResolveAuthInteractionResponse, ProductWorkflowError> {
        let run_id_hint = request.run_id_hint;
        let decision = request.decision.clone();
        self.resolutions.lock().expect("lock").push(request);
        let Some(run_id) = run_id_hint else {
            return Err(ProductWorkflowError::AuthInteractionRejected {
                kind: ironclaw_product_workflow::AuthInteractionRejectionKind::MissingAuth,
            });
        };
        Ok(match decision {
            AuthInteractionDecision::Deny => {
                ResolveAuthInteractionResponse::Canceled(CancelRunResponse {
                    run_id,
                    status: TurnStatus::Cancelled,
                    event_cursor: EventCursor(42),
                    already_terminal: false,
                    actor: None,
                })
            }
            AuthInteractionDecision::CredentialProvided { .. }
            | AuthInteractionDecision::CallbackCompleted { .. } => {
                ResolveAuthInteractionResponse::Resumed(ResumeTurnResponse {
                    run_id,
                    status: TurnStatus::Queued,
                    event_cursor: EventCursor(43),
                })
            }
        })
    }
}

/// A fake [`DeliveredGateRouteStore`] that always returns a fixed list of
/// records on `load_delivered_gate_route_by_conversation_fingerprint`,
/// regardless of the query key. Used only in tests that need the ambiguous-route
/// path, since the in-memory store deduplicates by `(tenant, user, gate_ref)`.
struct TwoRecordDeliveredGateRouteStore {
    records: Vec<ironclaw_outbound::DeliveredGateRouteRecord>,
    captured_args: std::sync::Mutex<
        Vec<(
            ironclaw_host_api::TenantId,
            ironclaw_host_api::UserId,
            String,
        )>,
    >,
}

impl TwoRecordDeliveredGateRouteStore {
    fn new(records: Vec<ironclaw_outbound::DeliveredGateRouteRecord>) -> Self {
        Self {
            records,
            captured_args: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn captured_args(
        &self,
    ) -> Vec<(
        ironclaw_host_api::TenantId,
        ironclaw_host_api::UserId,
        String,
    )> {
        self.captured_args.lock().expect("lock").clone()
    }
}

#[async_trait::async_trait]
impl ironclaw_outbound::DeliveredGateRouteStore for TwoRecordDeliveredGateRouteStore {
    async fn record_delivered_gate_route(
        &self,
        _record: ironclaw_outbound::DeliveredGateRouteRecord,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn load_delivered_gate_route(
        &self,
        _tenant_id: &ironclaw_host_api::TenantId,
        _user_id: &ironclaw_host_api::UserId,
        _gate_ref: &str,
    ) -> Result<Option<ironclaw_outbound::DeliveredGateRouteRecord>, String> {
        Ok(None)
    }

    async fn load_delivered_gate_route_by_conversation_fingerprint(
        &self,
        tenant_id: &ironclaw_host_api::TenantId,
        user_id: &ironclaw_host_api::UserId,
        conversation_fingerprint: &str,
    ) -> Result<Vec<ironclaw_outbound::DeliveredGateRouteRecord>, String> {
        self.captured_args.lock().expect("lock").push((
            tenant_id.clone(),
            user_id.clone(),
            conversation_fingerprint.to_string(),
        ));
        Ok(self.records.clone())
    }

    async fn remove_delivered_gate_route(
        &self,
        _tenant_id: &ironclaw_host_api::TenantId,
        _user_id: &ironclaw_host_api::UserId,
        _gate_ref: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn sweep_expired_delivered_gate_routes(
        &self,
        _now: chrono::DateTime<chrono::Utc>,
    ) -> Result<usize, String> {
        Ok(0)
    }
}

struct FailingRouteStore;

#[async_trait::async_trait]
impl ironclaw_outbound::DeliveredGateRouteStore for FailingRouteStore {
    async fn record_delivered_gate_route(
        &self,
        _record: ironclaw_outbound::DeliveredGateRouteRecord,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn load_delivered_gate_route(
        &self,
        _tenant_id: &ironclaw_host_api::TenantId,
        _user_id: &ironclaw_host_api::UserId,
        _gate_ref: &str,
    ) -> Result<Option<ironclaw_outbound::DeliveredGateRouteRecord>, String> {
        Ok(None)
    }

    async fn load_delivered_gate_route_by_conversation_fingerprint(
        &self,
        _tenant_id: &ironclaw_host_api::TenantId,
        _user_id: &ironclaw_host_api::UserId,
        _conversation_fingerprint: &str,
    ) -> Result<Vec<ironclaw_outbound::DeliveredGateRouteRecord>, String> {
        Err("store backend unavailable".to_string())
    }

    async fn remove_delivered_gate_route(
        &self,
        _tenant_id: &ironclaw_host_api::TenantId,
        _user_id: &ironclaw_host_api::UserId,
        _gate_ref: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn sweep_expired_delivered_gate_routes(
        &self,
        _now: chrono::DateTime<chrono::Utc>,
    ) -> Result<usize, String> {
        Ok(0)
    }
}

#[test]
fn action_fingerprint_retains_typed_identifiers() {
    let adapter_id = ProductAdapterId::new("test_adapter").expect("valid");
    let installation_id = AdapterInstallationId::new("install_alpha").expect("valid");
    let external_actor_ref =
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("valid actor");
    let source_binding_key = SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;")
        .expect("valid source binding key");
    let external_event_id = ExternalEventId::new("evt:typed").expect("valid");

    let fingerprint = ActionFingerprintKey::new(
        adapter_id.clone(),
        installation_id.clone(),
        external_actor_ref.clone(),
        source_binding_key.clone(),
        external_event_id.clone(),
    );

    assert_eq!(fingerprint.adapter_id, adapter_id);
    assert_eq!(fingerprint.installation_id, installation_id);
    assert_eq!(fingerprint.external_actor_ref, external_actor_ref);
    assert_eq!(fingerprint.source_binding_key, source_binding_key);
    assert_eq!(fingerprint.external_event_id, external_event_id);
}

#[test]
fn turn_submission_error_maps_to_stable_product_category() {
    let err: ProductAdapterError = ProductWorkflowError::TurnSubmissionFailed {
        error: TurnError::Unauthorized,
    }
    .into();

    match err {
        ProductAdapterError::WorkflowRejected {
            kind,
            status_code,
            retryable,
            ..
        } => {
            assert_eq!(kind, ProductWorkflowRejectionKind::Unauthorized);
            assert_eq!(status_code, 403);
            assert!(!retryable);
        }
        other => panic!("expected typed workflow rejection, got {other:?}"),
    }
}

#[test]
fn action_dispatch_kind_retains_typed_payload_refs() {
    let command_payload = ProductInboundPayload::Command(
        InboundCommandPayload::new("help", "", ProductTriggerReason::BotCommand).expect("valid"),
    );
    assert_eq!(
        ActionDispatchKind::try_from_payload(&command_payload).expect("command kind"),
        ActionDispatchKind::Command {
            command: ProductCommandName::new("help").expect("valid command")
        }
    );

    let gate_ref = LoopGateRef::new("gate:approval-1").expect("valid gate ref");
    let approval_payload = ProductInboundPayload::ApprovalResolution(
        ApprovalResolutionPayload::new(gate_ref.as_str(), ApprovalDecision::ApproveOnce)
            .expect("valid"),
    );
    assert_eq!(
        ActionDispatchKind::try_from_payload(&approval_payload).expect("approval kind"),
        ActionDispatchKind::ApprovalResolution { gate_ref }
    );

    let auth_payload = ProductInboundPayload::AuthResolution(
        AuthResolutionPayload::new("auth-request-1", AuthResolutionResult::Denied).expect("valid"),
    );
    assert_eq!(
        ActionDispatchKind::try_from_payload(&auth_payload).expect("auth kind"),
        ActionDispatchKind::AuthResolution {
            auth_request_ref: AuthRequestRef::new("auth-request-1").expect("valid auth ref")
        }
    );

    let linked_payload = ProductInboundPayload::LinkedThreadAction(
        LinkedThreadActionPayload::new("open-thread", None, None).expect("valid"),
    );
    assert_eq!(
        ActionDispatchKind::try_from_payload(&linked_payload).expect("linked kind"),
        ActionDispatchKind::LinkedThreadAction {
            action_id: LinkedThreadActionId::new("open-thread").expect("valid action id")
        }
    );
}

fn fake_binding() -> ResolvedBinding {
    ResolvedBinding {
        tenant_id: TenantId::new("tenant:fake").expect("valid tenant"),
        actor_user_id: UserId::new("user:fake").expect("valid actor user"),
        subject_user_id: Some(UserId::new("user:fake").expect("valid subject user")),
        thread_id: ThreadId::new("thread:fake").expect("valid thread"),
        agent_id: Some(AgentId::new("agent:fake").expect("valid agent")),
        project_id: None,
    }
}

#[derive(Default)]
struct ReplayCountingInboundTurnService {
    replay_attempts: Mutex<usize>,
    attempts: Mutex<usize>,
    accepted: Mutex<Vec<ProductInboundEnvelope>>,
}

impl ReplayCountingInboundTurnService {
    fn replay_attempt_count(&self) -> usize {
        *self
            .replay_attempts
            .lock()
            .expect("replay counter lock poisoned")
    }

    fn attempt_count(&self) -> usize {
        *self.attempts.lock().expect("attempt counter lock poisoned")
    }

    fn accepted_envelopes(&self) -> Vec<ProductInboundEnvelope> {
        self.accepted
            .lock()
            .expect("accepted envelopes lock poisoned")
            .clone()
    }

    fn accept_fresh_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<InboundTurnOutcome, ProductWorkflowError> {
        *self.attempts.lock().expect("attempt counter lock poisoned") += 1;
        self.accepted
            .lock()
            .expect("accepted envelopes lock poisoned")
            .push(envelope.clone());
        Ok(InboundTurnOutcome::Submitted {
            accepted_message_ref: AcceptedMessageRef::new(format!(
                "msg:{}",
                envelope.external_event_id()
            ))
            .expect("valid accepted message ref"),
            submitted_run_id: TurnRunId::new(),
            binding: fake_binding(),
        })
    }
}

#[async_trait]
impl InboundTurnService for ReplayCountingInboundTurnService {
    async fn replay_accepted_user_message(
        &self,
        _envelope: &ProductInboundEnvelope,
    ) -> Result<Option<InboundTurnOutcome>, ProductWorkflowError> {
        *self
            .replay_attempts
            .lock()
            .expect("replay counter lock poisoned") += 1;
        Ok(None)
    }

    async fn accept_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<InboundTurnOutcome, ProductWorkflowError> {
        if let Some(outcome) = self.replay_accepted_user_message(envelope).await? {
            return Ok(outcome);
        }
        self.accept_fresh_user_message(envelope)
    }

    async fn accept_user_message_with_before_policy(
        &self,
        envelope: &ProductInboundEnvelope,
        before_inbound_policy: &dyn BeforeInboundPolicy,
    ) -> Result<InboundUserMessageDispatch, ProductWorkflowError> {
        if let Some(outcome) = self.replay_accepted_user_message(envelope).await? {
            return Ok(InboundUserMessageDispatch::Accepted(outcome));
        }

        let ProductInboundPayload::UserMessage(payload) = envelope.payload() else {
            return Err(ProductWorkflowError::UnsupportedActionKind {
                kind: "non_user_message".into(),
            });
        };
        let policy_outcome = before_inbound_policy
            .check_user_message(BeforeInboundPolicyRequest::new(envelope, payload)?)
            .await?;
        let dispatch_envelope;
        let envelope_for_turn = match policy_outcome {
            BeforeInboundPolicyOutcome::Allow => envelope,
            BeforeInboundPolicyOutcome::RewriteUserMessage(payload) => {
                dispatch_envelope =
                    envelope.with_rewritten_user_message(payload).map_err(|_| {
                        ProductWorkflowError::TurnSubmissionRejected {
                            reason: "invalid policy-rewritten user message".into(),
                        }
                    })?;
                &dispatch_envelope
            }
            BeforeInboundPolicyOutcome::Reject(rejection) => {
                return Ok(InboundUserMessageDispatch::Rejected(rejection));
            }
            _ => {
                return Err(ProductWorkflowError::Transient {
                    reason: "unsupported before-inbound policy outcome".into(),
                });
            }
        };

        self.accept_fresh_user_message(envelope_for_turn)
            .map(InboundUserMessageDispatch::Accepted)
    }
}

fn fingerprint_actor() -> ExternalActorRef {
    ExternalActorRef::new("test", "user1", Option::<String>::None).expect("valid actor")
}

fn build_workflow() -> (
    DefaultProductWorkflow,
    Arc<FakeInboundTurnService>,
    Arc<FakeIdempotencyLedger>,
) {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone(), binding);
    (workflow, inbound, ledger)
}

fn build_workflow_with_policy() -> (
    DefaultProductWorkflow,
    Arc<FakeInboundTurnService>,
    Arc<FakeIdempotencyLedger>,
    Arc<FakeBeforeInboundPolicy>,
) {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let policy = Arc::new(FakeBeforeInboundPolicy::new());
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone(), binding)
        .with_before_inbound_policy(policy.clone());
    (workflow, inbound, ledger, policy)
}

fn build_workflow_with_binding() -> (
    DefaultProductWorkflow,
    Arc<FakeInboundTurnService>,
    Arc<FakeIdempotencyLedger>,
    Arc<FakeConversationBindingService>,
) {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone(), binding.clone());
    (workflow, inbound, ledger, binding)
}

fn scoped_approval_thread_reply_envelope(event_suffix: &str) -> ProductInboundEnvelope {
    sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("valid"),
        AdapterInstallationId::new("install_alpha").expect("valid"),
        ExternalEventId::new(format!("evt:{event_suffix}")).expect("valid"),
        ExternalActorRef::new("test", "user1", None::<String>).expect("actor"),
        ExternalConversationRef::new(
            None,
            "conv1",
            Some("delivered-gate-thread"),
            Some("reply-message"),
        )
        .expect("conversation"),
        ProductInboundPayload::ScopedApprovalResolution(
            ScopedApprovalResolutionPayload::new(ApprovalDecision::ApproveOnce)
                .expect("scoped approval payload"),
        ),
    )
}

fn explicit_approval_thread_reply_envelope(
    event_suffix: &str,
    gate_ref: &str,
) -> ProductInboundEnvelope {
    sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("valid"),
        AdapterInstallationId::new("install_alpha").expect("valid"),
        ExternalEventId::new(format!("evt:{event_suffix}")).expect("valid"),
        ExternalActorRef::new("test", "user1", None::<String>).expect("actor"),
        ExternalConversationRef::new(
            None,
            "conv1",
            Some("delivered-gate-thread"),
            Some("reply-message"),
        )
        .expect("conversation"),
        ProductInboundPayload::ApprovalResolution(
            ApprovalResolutionPayload::new(gate_ref, ApprovalDecision::ApproveOnce)
                .expect("approval payload"),
        ),
    )
}

fn auth_thread_reply_envelope(event_suffix: &str, gate_ref: &str) -> ProductInboundEnvelope {
    sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("valid"),
        AdapterInstallationId::new("install_alpha").expect("valid"),
        ExternalEventId::new(format!("evt:{event_suffix}")).expect("valid"),
        ExternalActorRef::new("test", "user1", None::<String>).expect("actor"),
        ExternalConversationRef::new(
            None,
            "conv1",
            Some("delivered-gate-thread"),
            Some("reply-message"),
        )
        .expect("conversation"),
        ProductInboundPayload::AuthResolution(
            AuthResolutionPayload::new(gate_ref, AuthResolutionResult::Denied)
                .expect("auth payload"),
        ),
    )
}

fn delivered_gate_thread_fingerprint() -> String {
    ironclaw_conversations::ExternalConversationRef::new(
        None,
        "conv1",
        Some("delivered-gate-thread"),
        None,
    )
    .expect("conversation route")
    .conversation_fingerprint()
}

async fn record_conversation_route_for_gate_ref(
    store: &dyn ironclaw_outbound::DeliveredGateRouteStore,
    gate_ref: &str,
    recorded_at: chrono::DateTime<Utc>,
) -> (TurnRunId, TurnScope) {
    let tenant_id = TenantId::new("tenant:install_alpha").expect("tenant");
    let user_id = UserId::new("user:user1").expect("user");
    let run_id = TurnRunId::new();
    let scope = TurnScope::new_with_owner(
        tenant_id.clone(),
        Some(AgentId::new("agent:delivered-route").expect("agent")),
        Some(ProjectId::new("project:delivered-route").expect("project")),
        ThreadId::new("thread:delivered-route-run").expect("thread"),
        Some(user_id.clone()),
    );
    store
        .record_delivered_gate_route(ironclaw_outbound::DeliveredGateRouteRecord {
            tenant_id,
            user_id,
            gate_ref: gate_ref.to_string(),
            run_id,
            scope: scope.clone(),
            recorded_at,
            delivered_conversation_fingerprints: vec![delivered_gate_thread_fingerprint()],
        })
        .await
        .expect("record delivered gate route");
    (run_id, scope)
}

async fn record_scoped_approval_conversation_route(
    store: &dyn ironclaw_outbound::DeliveredGateRouteStore,
    recorded_at: chrono::DateTime<Utc>,
) -> (GateRef, TurnRunId, TurnScope) {
    let gate_ref = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate ref");
    let (run_id, scope) =
        record_conversation_route_for_gate_ref(store, gate_ref.as_str(), recorded_at).await;
    (gate_ref, run_id, scope)
}

fn assert_scoped_approval_missing_gate(error: ProductAdapterError) {
    assert!(matches!(
        error,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::ScopeNotFound,
            status_code: 404,
            retryable: false,
            ..
        }
    ));
}

#[tokio::test]
async fn user_message_dispatches_through_inbound_turn_service() {
    let (workflow, inbound, ledger) = build_workflow();
    let envelope = sample_envelope("1");

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));
    assert_eq!(inbound.accepted_count(), 1);
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn approval_resolution_payload_routes_through_approval_interaction_service() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let gate_ref = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate ref");
    let run_id = TurnRunId::new();
    let approval_service = Arc::new(RecordingApprovalInteractionService::new(
        gate_ref.clone(),
        run_id,
    ));
    let workflow = DefaultProductWorkflow::new(inbound, ledger, binding)
        .with_approval_interaction_service(approval_service.clone());
    let envelope = sample_envelope_with_payload(
        "approval-resolution",
        ProductInboundPayload::ApprovalResolution(
            ApprovalResolutionPayload::new(gate_ref.as_str(), ApprovalDecision::ApproveOnce)
                .expect("approval payload"),
        ),
    );

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));
    let resolutions = approval_service.resolutions();
    assert_eq!(resolutions.len(), 1);
    assert_eq!(resolutions[0].gate_ref, gate_ref);
    assert_eq!(resolutions[0].run_id_hint, None);
    assert!(
        resolutions[0]
            .idempotency_key
            .as_str()
            .contains("approval-resolution")
    );
    assert_eq!(
        resolutions[0].decision,
        ApprovalInteractionDecision::ApproveOnce
    );
}

#[tokio::test]
async fn concrete_approval_resolution_rejects_unknown_installation_via_product_binding_service() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    let conversation_port: Arc<dyn ironclaw_conversations::ConversationBindingService> =
        conversations.clone();
    let binding = Arc::new(ProductConversationBindingService::new(
        conversation_port,
        StaticProductInstallationResolver::default(),
    ));
    let gate_ref = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate ref");
    let approval_service = Arc::new(RecordingApprovalInteractionService::new(
        gate_ref.clone(),
        TurnRunId::new(),
    ));
    let workflow = DefaultProductWorkflow::new(
        Arc::new(FakeInboundTurnService::new()),
        Arc::new(InMemoryIdempotencyLedger::new()),
        binding,
    )
    .with_approval_interaction_service(approval_service.clone());
    let envelope = sample_envelope_with_payload(
        "approval-unknown-installation",
        ProductInboundPayload::ApprovalResolution(
            ApprovalResolutionPayload::new(gate_ref.as_str(), ApprovalDecision::ApproveOnce)
                .expect("approval payload"),
        ),
    );

    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("unknown installation should reject before interaction dispatch");

    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::Unauthorized,
            status_code: 403,
            retryable: false,
            ..
        }
    ));
    assert!(approval_service.resolutions().is_empty());
}

#[tokio::test]
async fn auth_resolution_payload_routes_through_auth_interaction_service() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let gate_ref = GateRef::new("gate:auth-product").expect("auth gate ref");
    let run_id = TurnRunId::new();
    let credential_ref = CredentialAccountId::new();
    let auth_service = Arc::new(RecordingAuthInteractionService::new(
        gate_ref.clone(),
        run_id,
    ));
    let workflow = DefaultProductWorkflow::new(inbound, ledger, binding)
        .with_auth_interaction_service(auth_service.clone());
    let envelope = sample_envelope_with_payload(
        "auth-resolution",
        ProductInboundPayload::AuthResolution(
            AuthResolutionPayload::new(
                gate_ref.as_str(),
                AuthResolutionResult::CredentialProvided {
                    credential_ref: credential_ref.to_string(),
                },
            )
            .expect("auth payload"),
        ),
    );

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));
    let resolutions = auth_service.resolutions();
    assert_eq!(resolutions.len(), 1);
    assert_eq!(resolutions[0].gate_ref, gate_ref);
    assert_eq!(resolutions[0].run_id_hint, None);
    assert!(
        resolutions[0]
            .idempotency_key
            .as_str()
            .contains("auth-resolution")
    );
    assert_eq!(
        resolutions[0].decision,
        AuthInteractionDecision::CredentialProvided { credential_ref }
    );
}

#[tokio::test]
async fn auth_callback_and_denied_payloads_route_through_auth_interaction_service() {
    let callback_ref = AuthFlowId::new();
    for (event_suffix, result, expected) in [
        (
            "auth-callback-resolution",
            AuthResolutionResult::CallbackCompleted {
                callback_ref: callback_ref.to_string(),
            },
            AuthInteractionDecision::CallbackCompleted { callback_ref },
        ),
        (
            "auth-denied-resolution",
            AuthResolutionResult::Denied,
            AuthInteractionDecision::Deny,
        ),
    ] {
        let inbound = Arc::new(FakeInboundTurnService::new());
        let ledger = Arc::new(FakeIdempotencyLedger::new());
        let binding = Arc::new(FakeConversationBindingService::new());
        let gate_ref = GateRef::new(format!("gate:{event_suffix}")).expect("auth gate ref");
        let run_id = TurnRunId::new();
        let auth_service = Arc::new(RecordingAuthInteractionService::new(
            gate_ref.clone(),
            run_id,
        ));
        let workflow = DefaultProductWorkflow::new(inbound, ledger, binding)
            .with_auth_interaction_service(auth_service.clone());
        let envelope = sample_envelope_with_payload(
            event_suffix,
            ProductInboundPayload::AuthResolution(
                AuthResolutionPayload::new(gate_ref.as_str(), result).expect("auth payload"),
            ),
        );

        let ack = workflow.accept_inbound(envelope).await.expect("accept");

        assert!(matches!(ack, ProductInboundAck::Accepted { .. }));
        let resolutions = auth_service.resolutions();
        assert_eq!(resolutions.len(), 1);
        assert_eq!(resolutions[0].gate_ref, gate_ref);
        assert_eq!(resolutions[0].decision, expected);
    }
}

#[tokio::test]
async fn auth_deny_from_threaded_direct_prompt_uses_base_direct_binding() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            TenantId::new("tenant:alpha").expect("tenant"),
            ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter"),
            ironclaw_conversations::AdapterInstallationId::new("install_alpha")
                .expect("installation"),
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    let binding = product_binding_service(
        conversations,
        vec![(
            "test_adapter",
            "install_alpha",
            "tenant:alpha",
            "agent:alpha",
            Some("project:alpha"),
        )],
    );
    let base_envelope = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("installation"),
        ExternalEventId::new("evt:seed-direct").expect("event"),
        ExternalActorRef::new("test", "user1", None::<String>).expect("actor"),
        ExternalConversationRef::new(None, "conv1", None, None).expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("needs auth", vec![], ProductTriggerReason::DirectChat)
                .expect("message"),
        ),
    );
    let base_binding = binding
        .resolve_binding(ResolveBindingRequest::from_envelope(&base_envelope))
        .await
        .expect("seed base direct conversation binding");
    let gate_ref = GateRef::new("gate:auth-direct-thread").expect("auth gate");
    let auth_service = Arc::new(RecordingAuthInteractionService::new(
        gate_ref.clone(),
        TurnRunId::new(),
    ));
    let workflow = DefaultProductWorkflow::new(
        Arc::new(FakeInboundTurnService::new()),
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    )
    .with_auth_interaction_service(auth_service.clone());
    let threaded_deny = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("installation"),
        ExternalEventId::new("evt:threaded-auth-deny").expect("event"),
        ExternalActorRef::new("test", "user1", None::<String>).expect("actor"),
        ExternalConversationRef::new(None, "conv1", Some("prompt-thread-ts"), Some("reply-ts"))
            .expect("conversation"),
        ProductInboundPayload::AuthResolution(
            AuthResolutionPayload::new(gate_ref.as_str(), AuthResolutionResult::Denied)
                .expect("auth payload")
                .with_source_trigger(ProductTriggerReason::DirectChat),
        ),
    );

    let ack = workflow
        .accept_inbound(threaded_deny)
        .await
        .expect("threaded direct auth deny should use base binding");

    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));
    let resolutions = auth_service.resolutions();
    assert_eq!(resolutions.len(), 1);
    assert_eq!(resolutions[0].gate_ref, gate_ref);
    assert_eq!(resolutions[0].decision, AuthInteractionDecision::Deny);
    assert_eq!(resolutions[0].scope.thread_id, base_binding.thread_id);
}

#[tokio::test]
async fn approval_resolution_idempotency_key_is_stable_for_same_external_event() {
    let gate_ref = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate ref");
    let build = || {
        let inbound = Arc::new(FakeInboundTurnService::new());
        let ledger = Arc::new(FakeIdempotencyLedger::new());
        let binding = Arc::new(FakeConversationBindingService::new());
        let approval_service = Arc::new(RecordingApprovalInteractionService::new(
            gate_ref.clone(),
            TurnRunId::new(),
        ));
        let workflow = DefaultProductWorkflow::new(inbound, ledger, binding)
            .with_approval_interaction_service(approval_service.clone());
        (workflow, approval_service)
    };
    let envelope = || {
        sample_envelope_with_payload(
            "approval-resolution-stable",
            ProductInboundPayload::ApprovalResolution(
                ApprovalResolutionPayload::new(gate_ref.as_str(), ApprovalDecision::ApproveOnce)
                    .expect("approval payload"),
            ),
        )
    };
    let (workflow_a, approval_a) = build();
    let (workflow_b, approval_b) = build();

    workflow_a
        .accept_inbound(envelope())
        .await
        .expect("first accept");
    workflow_b
        .accept_inbound(envelope())
        .await
        .expect("second accept");

    assert_eq!(
        approval_a.resolutions()[0].idempotency_key,
        approval_b.resolutions()[0].idempotency_key
    );
}

#[tokio::test]
async fn approval_resolution_idempotency_key_ignores_actor_display_name() {
    let gate_ref = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate ref");
    let build = || {
        let inbound = Arc::new(FakeInboundTurnService::new());
        let ledger = Arc::new(FakeIdempotencyLedger::new());
        let binding = Arc::new(FakeConversationBindingService::new());
        let approval_service = Arc::new(RecordingApprovalInteractionService::new(
            gate_ref.clone(),
            TurnRunId::new(),
        ));
        let workflow = DefaultProductWorkflow::new(inbound, ledger, binding)
            .with_approval_interaction_service(approval_service.clone());
        (workflow, approval_service)
    };
    let envelope = |display_name| {
        sample_envelope_with_context(
            ProductAdapterId::new("test_adapter").expect("valid"),
            AdapterInstallationId::new("install_alpha").expect("valid"),
            ExternalEventId::new("evt:approval-display-name").expect("valid"),
            ExternalActorRef::new("test", "user1", Some(display_name)).expect("valid actor"),
            ExternalConversationRef::new(None, "conv1", None, None).expect("valid conversation"),
            ProductInboundPayload::ApprovalResolution(
                ApprovalResolutionPayload::new(gate_ref.as_str(), ApprovalDecision::ApproveOnce)
                    .expect("approval payload"),
            ),
        )
    };
    let (workflow_a, approval_a) = build();
    let (workflow_b, approval_b) = build();

    workflow_a
        .accept_inbound(envelope("Alice A."))
        .await
        .expect("first accept");
    workflow_b
        .accept_inbound(envelope("Alice B."))
        .await
        .expect("second accept");

    assert_eq!(
        approval_a.resolutions()[0].idempotency_key,
        approval_b.resolutions()[0].idempotency_key
    );
}

#[tokio::test]
async fn approval_resolution_deny_routes_through_approval_interaction_service() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let gate_ref = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate ref");
    let approval_service = Arc::new(RecordingApprovalInteractionService::new(
        gate_ref.clone(),
        TurnRunId::new(),
    ));
    let workflow = DefaultProductWorkflow::new(inbound, ledger, binding)
        .with_approval_interaction_service(approval_service.clone());
    let envelope = sample_envelope_with_payload(
        "approval-deny",
        ProductInboundPayload::ApprovalResolution(
            ApprovalResolutionPayload::new(gate_ref.as_str(), ApprovalDecision::Deny)
                .expect("approval payload"),
        ),
    );

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));
    let resolutions = approval_service.resolutions();
    assert_eq!(resolutions.len(), 1);
    assert_eq!(resolutions[0].gate_ref, gate_ref);
    assert_eq!(resolutions[0].run_id_hint, None);
    assert_eq!(resolutions[0].decision, ApprovalInteractionDecision::Deny);
}

#[tokio::test]
async fn approval_resolution_always_allow_routes_through_approval_interaction_service() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let gate_ref = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate ref");
    let approval_service = Arc::new(RecordingApprovalInteractionService::new(
        gate_ref.clone(),
        TurnRunId::new(),
    ));
    let workflow = DefaultProductWorkflow::new(inbound, ledger, binding)
        .with_approval_interaction_service(approval_service.clone());
    let envelope = sample_envelope_with_payload(
        "approval-always-allow",
        ProductInboundPayload::ApprovalResolution(
            ApprovalResolutionPayload::new(gate_ref.as_str(), ApprovalDecision::AlwaysAllow)
                .expect("approval payload"),
        ),
    );

    workflow
        .accept_inbound(envelope)
        .await
        .expect("always allow routes through approval interaction");

    let resolutions = approval_service.resolutions();
    assert_eq!(resolutions.len(), 1);
    assert_eq!(
        resolutions[0].decision,
        ApprovalInteractionDecision::AlwaysAllow
    );
}

#[tokio::test]
async fn scoped_approval_resolution_rejects_ambiguous_gate() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let first_gate = approval_gate_ref(ApprovalRequestId::new()).expect("first gate ref");
    let second_gate = approval_gate_ref(ApprovalRequestId::new()).expect("second gate ref");
    let approval_service = Arc::new(RecordingApprovalInteractionService::with_pending(vec![
        (first_gate, TurnRunId::new()),
        (second_gate, TurnRunId::new()),
    ]));
    let workflow = DefaultProductWorkflow::new(inbound, ledger, binding)
        .with_approval_interaction_service(approval_service.clone());
    let envelope = sample_envelope_with_payload(
        "scoped-approval-ambiguous",
        ProductInboundPayload::ScopedApprovalResolution(
            ScopedApprovalResolutionPayload::new(ApprovalDecision::ApproveOnce)
                .expect("scoped approval payload"),
        ),
    );

    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("ambiguous gate should reject before interaction dispatch");

    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::Ambiguous,
            status_code: 409,
            retryable: false,
            ..
        }
    ));
    assert!(approval_service.resolutions().is_empty());
}

#[tokio::test]
async fn scoped_approval_resolves_via_conversation_route() {
    let route_store: Arc<dyn ironclaw_outbound::DeliveredGateRouteStore> =
        Arc::new(ironclaw_outbound::InMemoryDeliveredGateRouteStore::default());
    let (gate_ref, run_id, route_scope) =
        record_scoped_approval_conversation_route(route_store.as_ref(), Utc::now()).await;
    let approval_service = Arc::new(RecordingApprovalInteractionService::with_pending(Vec::new()));
    let workflow = DefaultProductWorkflow::new(
        Arc::new(FakeInboundTurnService::new()),
        Arc::new(FakeIdempotencyLedger::new()),
        Arc::new(FakeConversationBindingService::new()),
    )
    .with_approval_interaction_service(approval_service.clone())
    .with_delivered_gate_routes(route_store);

    let ack = workflow
        .accept_inbound(scoped_approval_thread_reply_envelope(
            "scoped-approval-conversation-route",
        ))
        .await
        .expect("scoped approval should resolve through delivered conversation route");

    assert!(matches!(
        ack,
        ProductInboundAck::Accepted {
            submitted_run_id,
            ..
        } if submitted_run_id == run_id
    ));
    let resolutions = approval_service.resolutions();
    assert_eq!(resolutions.len(), 1);
    assert_eq!(resolutions[0].gate_ref, gate_ref);
    assert_eq!(resolutions[0].run_id_hint, Some(run_id));
    assert_eq!(resolutions[0].scope.thread_id, route_scope.thread_id);
    assert_eq!(
        resolutions[0].decision,
        ApprovalInteractionDecision::ApproveOnce
    );
}

#[tokio::test]
async fn scoped_approval_misses_if_route_expired() {
    let route_store: Arc<dyn ironclaw_outbound::DeliveredGateRouteStore> =
        Arc::new(ironclaw_outbound::InMemoryDeliveredGateRouteStore::default());
    record_scoped_approval_conversation_route(
        route_store.as_ref(),
        Utc::now() - ironclaw_outbound::DELIVERED_GATE_ROUTE_TTL - Duration::seconds(1),
    )
    .await;
    let approval_service = Arc::new(RecordingApprovalInteractionService::with_pending(Vec::new()));
    let workflow = DefaultProductWorkflow::new(
        Arc::new(FakeInboundTurnService::new()),
        Arc::new(FakeIdempotencyLedger::new()),
        Arc::new(FakeConversationBindingService::new()),
    )
    .with_approval_interaction_service(approval_service.clone())
    .with_delivered_gate_routes(route_store);

    let error = workflow
        .accept_inbound(scoped_approval_thread_reply_envelope(
            "scoped-approval-expired-route",
        ))
        .await
        .expect_err("expired delivered route should fall through to missing gate");

    assert_scoped_approval_missing_gate(error);
    assert!(approval_service.resolutions().is_empty());
}

#[tokio::test]
async fn scoped_approval_misses_if_no_route() {
    let route_store: Arc<dyn ironclaw_outbound::DeliveredGateRouteStore> =
        Arc::new(ironclaw_outbound::InMemoryDeliveredGateRouteStore::default());
    let approval_service = Arc::new(RecordingApprovalInteractionService::with_pending(Vec::new()));
    let workflow = DefaultProductWorkflow::new(
        Arc::new(FakeInboundTurnService::new()),
        Arc::new(FakeIdempotencyLedger::new()),
        Arc::new(FakeConversationBindingService::new()),
    )
    .with_approval_interaction_service(approval_service.clone())
    .with_delivered_gate_routes(route_store);

    let error = workflow
        .accept_inbound(scoped_approval_thread_reply_envelope(
            "scoped-approval-no-route",
        ))
        .await
        .expect_err("missing delivered route should fall through to missing gate");

    assert_scoped_approval_missing_gate(error);
    assert!(approval_service.resolutions().is_empty());
}

#[tokio::test]
async fn scoped_approval_delivered_route_store_error_falls_back_to_missing_gate() {
    let route_store: Arc<dyn ironclaw_outbound::DeliveredGateRouteStore> =
        Arc::new(FailingRouteStore);
    let approval_service = Arc::new(RecordingApprovalInteractionService::with_pending(Vec::new()));
    let workflow = DefaultProductWorkflow::new(
        Arc::new(FakeInboundTurnService::new()),
        Arc::new(FakeIdempotencyLedger::new()),
        Arc::new(FakeConversationBindingService::new()),
    )
    .with_approval_interaction_service(approval_service.clone())
    .with_delivered_gate_routes(route_store);

    let err = workflow
        .accept_inbound(scoped_approval_thread_reply_envelope(
            "store-error-fallback",
        ))
        .await
        .expect_err("store errors should fall through to missing gate");

    assert_scoped_approval_missing_gate(err);
    assert!(approval_service.resolutions().is_empty());
}

#[tokio::test]
async fn scoped_approval_missing_gate_fallback_reuses_dispatcher_binding() {
    // The MissingGate fallback must reuse the binding the dispatcher already
    // resolved, not re-derive a topic-stripped base binding. Program the two
    // lookups to diverge: the thread-scoped binding matches the route owner,
    // the base (topic-stripped) binding belongs to a different actor. Only a
    // fallback that reuses the dispatcher binding can resolve the gate.
    let route_store: Arc<dyn ironclaw_outbound::DeliveredGateRouteStore> =
        Arc::new(ironclaw_outbound::InMemoryDeliveredGateRouteStore::default());
    let (gate_ref, run_id, _route_scope) =
        record_scoped_approval_conversation_route(route_store.as_ref(), Utc::now()).await;
    let binding_service = Arc::new(FakeConversationBindingService::new());
    let owner_binding = ResolvedBinding {
        tenant_id: TenantId::new("tenant:install_alpha").expect("tenant"),
        actor_user_id: UserId::new("user:user1").expect("actor"),
        subject_user_id: Some(UserId::new("user:user1").expect("subject")),
        thread_id: ThreadId::new("thread:dm-topic").expect("thread"),
        agent_id: Some(AgentId::new("agent:fake").expect("agent")),
        project_id: None,
    };
    let divergent_base_binding = ResolvedBinding {
        actor_user_id: UserId::new("user:someone-else").expect("actor"),
        subject_user_id: Some(UserId::new("user:someone-else").expect("subject")),
        ..owner_binding.clone()
    };
    let thread_ref = ExternalConversationRef::new(
        None::<&str>,
        "conv1",
        Some("delivered-gate-thread"),
        None::<&str>,
    )
    .expect("thread ref");
    let base_ref = ExternalConversationRef::new(None::<&str>, "conv1", None::<&str>, None::<&str>)
        .expect("base ref");
    binding_service.program_binding(thread_ref.conversation_fingerprint(), owner_binding);
    binding_service.program_binding(base_ref.conversation_fingerprint(), divergent_base_binding);
    let approval_service = Arc::new(RecordingApprovalInteractionService::with_pending(Vec::new()));
    let workflow = DefaultProductWorkflow::new(
        Arc::new(FakeInboundTurnService::new()),
        Arc::new(FakeIdempotencyLedger::new()),
        binding_service.clone(),
    )
    .with_approval_interaction_service(approval_service.clone())
    .with_delivered_gate_routes(route_store);

    let ack = workflow
        .accept_inbound(scoped_approval_thread_reply_envelope(
            "scoped-approval-binding-reuse",
        ))
        .await
        .expect("fallback must resolve using the dispatcher's binding");

    assert!(matches!(
        ack,
        ProductInboundAck::Accepted {
            submitted_run_id,
            ..
        } if submitted_run_id == run_id
    ));
    let resolutions = approval_service.resolutions();
    assert_eq!(resolutions.len(), 1);
    assert_eq!(resolutions[0].gate_ref, gate_ref);
    assert_eq!(
        resolutions[0].actor,
        TurnActor::new(UserId::new("user:user1").expect("actor")),
        "resolution must act as the dispatcher-bound actor, not the re-derived base binding"
    );
    assert_eq!(
        binding_service.resolve_count(),
        1,
        "fallback must not perform a second binding lookup"
    );
}

#[tokio::test]
async fn auth_resolution_resolves_via_conversation_route_after_missing_auth() {
    let route_store: Arc<dyn ironclaw_outbound::DeliveredGateRouteStore> =
        Arc::new(ironclaw_outbound::InMemoryDeliveredGateRouteStore::default());
    let gate_ref = GateRef::new("gate:auth-conversation-route").expect("auth gate ref");
    let (run_id, route_scope) =
        record_conversation_route_for_gate_ref(route_store.as_ref(), gate_ref.as_str(), Utc::now())
            .await;
    let auth_service = Arc::new(MissingAuthThenRecordingAuthService::default());
    let workflow = DefaultProductWorkflow::new(
        Arc::new(FakeInboundTurnService::new()),
        Arc::new(FakeIdempotencyLedger::new()),
        Arc::new(FakeConversationBindingService::new()),
    )
    .with_auth_interaction_service(auth_service.clone())
    .with_delivered_gate_routes(route_store);

    let ack = workflow
        .accept_inbound(auth_thread_reply_envelope(
            "auth-conversation-route",
            gate_ref.as_str(),
        ))
        .await
        .expect("auth should resolve through delivered conversation route");

    assert!(matches!(
        ack,
        ProductInboundAck::Accepted {
            submitted_run_id,
            ..
        } if submitted_run_id == run_id
    ));
    let resolutions = auth_service.resolutions();
    assert_eq!(resolutions.len(), 2);
    assert_eq!(resolutions[0].run_id_hint, None);
    assert_eq!(resolutions[1].gate_ref, gate_ref);
    assert_eq!(resolutions[1].run_id_hint, Some(run_id));
    assert_eq!(resolutions[1].scope.thread_id, route_scope.thread_id);
    assert_eq!(resolutions[1].decision, AuthInteractionDecision::Deny);
}

#[tokio::test]
async fn explicit_approval_delivered_route_requires_gate_ref_match() {
    let route_store: Arc<dyn ironclaw_outbound::DeliveredGateRouteStore> =
        Arc::new(ironclaw_outbound::InMemoryDeliveredGateRouteStore::default());
    let route_gate_ref = GateRef::new("gate:approval-route-match").expect("gate ref");
    let payload_gate_ref = GateRef::new("gate:approval-route-mismatch").expect("gate ref");
    record_conversation_route_for_gate_ref(
        route_store.as_ref(),
        route_gate_ref.as_str(),
        Utc::now(),
    )
    .await;
    let approval_service = Arc::new(MissingGateThenRecordingApprovalService::default());
    let workflow = DefaultProductWorkflow::new(
        Arc::new(FakeInboundTurnService::new()),
        Arc::new(FakeIdempotencyLedger::new()),
        Arc::new(FakeConversationBindingService::new()),
    )
    .with_approval_interaction_service(approval_service.clone())
    .with_delivered_gate_routes(route_store);

    let error = workflow
        .accept_inbound(explicit_approval_thread_reply_envelope(
            "approval-explicit-route-mismatch",
            payload_gate_ref.as_str(),
        ))
        .await
        .expect_err("mismatched explicit approval ref must not use delivered route");

    assert_scoped_approval_missing_gate(error);
    let resolutions = approval_service.resolutions();
    assert_eq!(resolutions.len(), 1);
    assert_eq!(resolutions[0].gate_ref, payload_gate_ref);
    assert_eq!(resolutions[0].run_id_hint, None);
}

#[tokio::test]
async fn explicit_auth_delivered_route_requires_gate_ref_match() {
    let route_store: Arc<dyn ironclaw_outbound::DeliveredGateRouteStore> =
        Arc::new(ironclaw_outbound::InMemoryDeliveredGateRouteStore::default());
    let route_gate_ref = GateRef::new("gate:auth-route-match").expect("gate ref");
    let payload_gate_ref = GateRef::new("gate:auth-route-mismatch").expect("gate ref");
    record_conversation_route_for_gate_ref(
        route_store.as_ref(),
        route_gate_ref.as_str(),
        Utc::now(),
    )
    .await;
    let auth_service = Arc::new(MissingAuthThenRecordingAuthService::default());
    let workflow = DefaultProductWorkflow::new(
        Arc::new(FakeInboundTurnService::new()),
        Arc::new(FakeIdempotencyLedger::new()),
        Arc::new(FakeConversationBindingService::new()),
    )
    .with_auth_interaction_service(auth_service.clone())
    .with_delivered_gate_routes(route_store);

    let error = workflow
        .accept_inbound(auth_thread_reply_envelope(
            "auth-explicit-route-mismatch",
            payload_gate_ref.as_str(),
        ))
        .await
        .expect_err("mismatched explicit auth ref must not use delivered route");

    assert_scoped_approval_missing_gate(error);
    let resolutions = auth_service.resolutions();
    assert_eq!(resolutions.len(), 1);
    assert_eq!(resolutions[0].gate_ref, payload_gate_ref);
    assert_eq!(resolutions[0].run_id_hint, None);
}

/// Two live routes share the same conversation fingerprint. A bare scoped-approval
/// reply (no gate_ref) must not pick one arbitrarily — it must reject with
/// `Ambiguous` (409) before any approval-interaction dispatch occurs.
#[tokio::test]
async fn scoped_approval_two_live_routes_same_conversation_rejects_ambiguous() {
    let tenant_id = TenantId::new("tenant:install_alpha").expect("tenant");
    let user_id = UserId::new("user:user1").expect("user");
    let fingerprint = delivered_gate_thread_fingerprint();
    let make_record = |gate_ref: &str| ironclaw_outbound::DeliveredGateRouteRecord {
        tenant_id: tenant_id.clone(),
        user_id: user_id.clone(),
        gate_ref: gate_ref.to_string(),
        run_id: TurnRunId::new(),
        scope: TurnScope::new_with_owner(
            tenant_id.clone(),
            Some(AgentId::new("agent:delivered-route").expect("agent")),
            Some(ProjectId::new("project:delivered-route").expect("project")),
            ThreadId::new("thread:delivered-route-run").expect("thread"),
            Some(user_id.clone()),
        ),
        recorded_at: Utc::now(),
        delivered_conversation_fingerprints: vec![fingerprint.clone()],
    };
    let route_store = Arc::new(TwoRecordDeliveredGateRouteStore::new(vec![
        make_record(
            approval_gate_ref(ApprovalRequestId::new())
                .expect("gate")
                .as_str(),
        ),
        make_record(
            approval_gate_ref(ApprovalRequestId::new())
                .expect("gate")
                .as_str(),
        ),
    ]));
    let approval_service = Arc::new(RecordingApprovalInteractionService::with_pending(Vec::new()));
    let workflow = DefaultProductWorkflow::new(
        Arc::new(FakeInboundTurnService::new()),
        Arc::new(FakeIdempotencyLedger::new()),
        Arc::new(FakeConversationBindingService::new()),
    )
    .with_approval_interaction_service(approval_service.clone())
    .with_delivered_gate_routes(route_store.clone());

    let err = workflow
        .accept_inbound(scoped_approval_thread_reply_envelope(
            "two-live-routes-ambiguous",
        ))
        .await
        .expect_err("two live routes must reject as ambiguous");

    assert!(
        matches!(
            err,
            ProductAdapterError::WorkflowRejected {
                kind: ProductWorkflowRejectionKind::Ambiguous,
                status_code: 409,
                retryable: false,
                ..
            }
        ),
        "expected Ambiguous/409 for ambiguous delivered route, got: {err:?}"
    );
    assert!(
        approval_service.resolutions().is_empty(),
        "approval service must not be consulted on ambiguous route"
    );
    // The binding resolves `ExternalActorRef("test", "user1")` → actor user
    // `"user:user1"` via FakeConversationBindingService (see fakes.rs actor
    // derivation: `"user:" + external_actor_ref.id()`).
    assert_eq!(
        route_store.captured_args(),
        vec![(
            tenant_id,
            UserId::new("user:user1").expect("actor user"),
            fingerprint
        )],
        "conversation fallback must query by tenant, user, and delivered conversation fingerprint"
    );
}

/// One expired route + one live route sharing the same conversation fingerprint.
/// The expired one must be filtered out; the live one must resolve normally.
#[tokio::test]
async fn scoped_approval_one_expired_one_live_resolves_live() {
    let route_store: Arc<dyn ironclaw_outbound::DeliveredGateRouteStore> =
        Arc::new(ironclaw_outbound::InMemoryDeliveredGateRouteStore::default());
    // Record an expired route first.
    record_scoped_approval_conversation_route(
        route_store.as_ref(),
        Utc::now() - ironclaw_outbound::DELIVERED_GATE_ROUTE_TTL - Duration::seconds(1),
    )
    .await;
    // Record a fresh live route.
    let (gate_ref, run_id, route_scope) =
        record_scoped_approval_conversation_route(route_store.as_ref(), Utc::now()).await;
    let approval_service = Arc::new(RecordingApprovalInteractionService::with_pending(Vec::new()));
    let workflow = DefaultProductWorkflow::new(
        Arc::new(FakeInboundTurnService::new()),
        Arc::new(FakeIdempotencyLedger::new()),
        Arc::new(FakeConversationBindingService::new()),
    )
    .with_approval_interaction_service(approval_service.clone())
    .with_delivered_gate_routes(route_store);

    let ack = workflow
        .accept_inbound(scoped_approval_thread_reply_envelope(
            "expired-plus-live-resolves",
        ))
        .await
        .expect("expired route must be filtered; live route must resolve");

    assert!(
        matches!(
            ack,
            ProductInboundAck::Accepted {
                submitted_run_id,
                ..
            } if submitted_run_id == run_id
        ),
        "expected run_id from the live route"
    );
    let resolutions = approval_service.resolutions();
    assert_eq!(resolutions.len(), 1);
    assert_eq!(resolutions[0].gate_ref, gate_ref);
    assert_eq!(resolutions[0].run_id_hint, Some(run_id));
    assert_eq!(resolutions[0].scope.thread_id, route_scope.thread_id);
}

/// A route stored for a different actor (user2) sharing the same conversation
/// fingerprint must be invisible to the user1 reply — actor mismatch must
/// filter it out so the resolution falls through to the interaction service.
#[tokio::test]
async fn scoped_approval_actor_mismatch_filtered_out() {
    let route_store: Arc<dyn ironclaw_outbound::DeliveredGateRouteStore> =
        Arc::new(ironclaw_outbound::InMemoryDeliveredGateRouteStore::default());
    // Record a route owned by user:user2 — different actor than the envelope's user1.
    let tenant_id = TenantId::new("tenant:install_alpha").expect("tenant");
    let other_user_id = UserId::new("user:user2").expect("other user");
    let gate_ref_other =
        approval_gate_ref(ApprovalRequestId::new()).expect("approval gate ref for other user");
    route_store
        .record_delivered_gate_route(ironclaw_outbound::DeliveredGateRouteRecord {
            tenant_id: tenant_id.clone(),
            user_id: other_user_id.clone(),
            gate_ref: gate_ref_other.as_str().to_string(),
            run_id: TurnRunId::new(),
            scope: TurnScope::new_with_owner(
                tenant_id,
                Some(AgentId::new("agent:delivered-route").expect("agent")),
                Some(ProjectId::new("project:delivered-route").expect("project")),
                ThreadId::new("thread:other-user-route").expect("thread"),
                Some(other_user_id),
            ),
            recorded_at: Utc::now(),
            delivered_conversation_fingerprints: vec![
                ironclaw_conversations::ExternalConversationRef::new(
                    None,
                    "conv1",
                    Some("delivered-gate-thread"),
                    None,
                )
                .expect("conversation route")
                .conversation_fingerprint(),
            ],
        })
        .await
        .expect("record route for other user");

    let approval_service = Arc::new(RecordingApprovalInteractionService::with_pending(Vec::new()));
    let workflow = DefaultProductWorkflow::new(
        Arc::new(FakeInboundTurnService::new()),
        Arc::new(FakeIdempotencyLedger::new()),
        Arc::new(FakeConversationBindingService::new()),
    )
    .with_approval_interaction_service(approval_service.clone())
    .with_delivered_gate_routes(route_store);

    let err = workflow
        .accept_inbound(scoped_approval_thread_reply_envelope(
            "actor-mismatch-filtered",
        ))
        .await
        .expect_err("actor-mismatched route must be filtered; falls through to missing gate");

    assert_scoped_approval_missing_gate(err);
    assert!(
        approval_service.resolutions().is_empty(),
        "approval service must not be consulted when route filtered by actor"
    );
}

/// Explicit approval with a gate_ref that matches no stored delivered route must
/// fall through to the interaction service with the original gate_ref (no
/// cross-contamination from a different route in the same conversation).
#[tokio::test]
async fn explicit_approval_gate_ref_mismatch_leaves_original_rejection() {
    let route_store: Arc<dyn ironclaw_outbound::DeliveredGateRouteStore> =
        Arc::new(ironclaw_outbound::InMemoryDeliveredGateRouteStore::default());
    let route_gate_ref = GateRef::new("gate:approval-stored-ref").expect("stored gate ref");
    let payload_gate_ref = GateRef::new("gate:approval-payload-ref").expect("payload gate ref");
    record_conversation_route_for_gate_ref(
        route_store.as_ref(),
        route_gate_ref.as_str(),
        Utc::now(),
    )
    .await;
    let approval_service = Arc::new(MissingGateThenRecordingApprovalService::default());
    let workflow = DefaultProductWorkflow::new(
        Arc::new(FakeInboundTurnService::new()),
        Arc::new(FakeIdempotencyLedger::new()),
        Arc::new(FakeConversationBindingService::new()),
    )
    .with_approval_interaction_service(approval_service.clone())
    .with_delivered_gate_routes(route_store);

    let err = workflow
        .accept_inbound(explicit_approval_thread_reply_envelope(
            "explicit-mismatch-original-rejection",
            payload_gate_ref.as_str(),
        ))
        .await
        .expect_err("gate_ref mismatch must miss the stored route and fall through");

    assert_scoped_approval_missing_gate(err);
    let resolutions = approval_service.resolutions();
    assert_eq!(
        resolutions.len(),
        1,
        "interaction service must be invoked once"
    );
    assert_eq!(
        resolutions[0].gate_ref, payload_gate_ref,
        "interaction must receive the original payload gate_ref unchanged"
    );
    assert_eq!(
        resolutions[0].run_id_hint, None,
        "no run_id_hint when route missed"
    );
}

/// Two live auth routes share the same conversation fingerprint and the same
/// gate_ref. Because `RouteKey = (tenant, user, gate_ref)` deduplicates in the
/// in-memory store, we instead use a store that always returns two synthetic
/// records to exercise the `AmbiguousAuth` path in the workflow.
///
/// The workflow must reject with `Ambiguous` (409) after the one pre-fallback
/// auth-service call and must not make a second auth interaction service call.
#[tokio::test]
async fn auth_two_live_routes_same_conversation_rejects_ambiguous() {
    // Build two synthetic records that both pass all filters (same tenant, same
    // actor, same gate_ref, not expired) but differ only in run_id + scope.
    let tenant_id = TenantId::new("tenant:install_alpha").expect("tenant");
    let user_id = UserId::new("user:user1").expect("user");
    let shared_gate_ref = GateRef::new("gate:auth-ambiguous-shared").expect("shared gate ref");
    let make_record = |run_id: TurnRunId| ironclaw_outbound::DeliveredGateRouteRecord {
        tenant_id: tenant_id.clone(),
        user_id: user_id.clone(),
        gate_ref: shared_gate_ref.as_str().to_string(),
        run_id,
        scope: TurnScope::new_with_owner(
            tenant_id.clone(),
            Some(AgentId::new("agent:delivered-route").expect("agent")),
            Some(ProjectId::new("project:delivered-route").expect("project")),
            ThreadId::new("thread:delivered-route-run").expect("thread"),
            Some(user_id.clone()),
        ),
        recorded_at: Utc::now(),
        delivered_conversation_fingerprints: vec![
            ironclaw_conversations::ExternalConversationRef::new(
                None,
                "conv1",
                Some("delivered-gate-thread"),
                None,
            )
            .expect("conversation ref")
            .conversation_fingerprint(),
        ],
    };
    let expected_fingerprint = delivered_gate_thread_fingerprint();
    let route_store = Arc::new(TwoRecordDeliveredGateRouteStore::new(vec![
        make_record(TurnRunId::new()),
        make_record(TurnRunId::new()),
    ]));
    let auth_service = Arc::new(MissingAuthThenRecordingAuthService::default());
    let workflow = DefaultProductWorkflow::new(
        Arc::new(FakeInboundTurnService::new()),
        Arc::new(FakeIdempotencyLedger::new()),
        Arc::new(FakeConversationBindingService::new()),
    )
    .with_auth_interaction_service(auth_service.clone())
    .with_delivered_gate_routes(route_store.clone());

    let err = workflow
        .accept_inbound(auth_thread_reply_envelope(
            "auth-two-live-ambiguous",
            shared_gate_ref.as_str(),
        ))
        .await
        .expect_err("two live auth routes must reject as ambiguous");

    assert!(
        matches!(
            err,
            ProductAdapterError::WorkflowRejected {
                kind: ProductWorkflowRejectionKind::Ambiguous,
                status_code: 409,
                retryable: false,
                ..
            }
        ),
        "expected Ambiguous/409 for ambiguous auth delivered route, got: {err:?}"
    );
    // The auth service receives one initial call (the normal-path attempt that
    // returns MissingAuth), which then triggers the delivered-route fallback
    // where ambiguity is detected. The key invariant is that no second service
    // call is made — the ambiguous result is returned directly without any
    // further dispatch.
    let resolutions = auth_service.resolutions();
    assert_eq!(
        resolutions.len(),
        1,
        "auth service must receive exactly one (pre-ambiguity) call, got {resolutions:?}"
    );
    assert_eq!(
        resolutions[0].run_id_hint, None,
        "the pre-ambiguity call must not have a run_id_hint from a delivered route"
    );
    // The binding resolves `ExternalActorRef("test", "user1")` → actor user
    // `"user:user1"` via FakeConversationBindingService (see fakes.rs actor
    // derivation: `"user:" + external_actor_ref.id()`).
    assert_eq!(
        route_store.captured_args(),
        vec![(
            tenant_id,
            UserId::new("user:user1").expect("actor user"),
            expected_fingerprint
        )],
        "auth fallback must query by tenant, user, and delivered conversation fingerprint"
    );
}

/// Explicit auth with a gate_ref that matches no stored delivered route must
/// fall through to the interaction service with the original gate_ref.
#[tokio::test]
async fn explicit_auth_gate_ref_mismatch_leaves_original_rejection() {
    let route_store: Arc<dyn ironclaw_outbound::DeliveredGateRouteStore> =
        Arc::new(ironclaw_outbound::InMemoryDeliveredGateRouteStore::default());
    let route_gate_ref = GateRef::new("gate:auth-stored-ref").expect("stored gate ref");
    let payload_gate_ref = GateRef::new("gate:auth-payload-ref").expect("payload gate ref");
    record_conversation_route_for_gate_ref(
        route_store.as_ref(),
        route_gate_ref.as_str(),
        Utc::now(),
    )
    .await;
    let auth_service = Arc::new(MissingAuthThenRecordingAuthService::default());
    let workflow = DefaultProductWorkflow::new(
        Arc::new(FakeInboundTurnService::new()),
        Arc::new(FakeIdempotencyLedger::new()),
        Arc::new(FakeConversationBindingService::new()),
    )
    .with_auth_interaction_service(auth_service.clone())
    .with_delivered_gate_routes(route_store);

    let err = workflow
        .accept_inbound(auth_thread_reply_envelope(
            "auth-explicit-mismatch-original-rejection",
            payload_gate_ref.as_str(),
        ))
        .await
        .expect_err("auth gate_ref mismatch must miss the stored route and fall through");

    assert_scoped_approval_missing_gate(err);
    let resolutions = auth_service.resolutions();
    assert_eq!(resolutions.len(), 1, "auth service must be invoked once");
    assert_eq!(
        resolutions[0].gate_ref, payload_gate_ref,
        "auth service must receive the original payload gate_ref unchanged"
    );
    assert_eq!(
        resolutions[0].run_id_hint, None,
        "no run_id_hint when route missed"
    );
}

#[tokio::test]
async fn approval_resolution_without_interaction_service_returns_retryable_unavailable() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let workflow = DefaultProductWorkflow::new(inbound, ledger, binding);
    let gate_ref = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate ref");
    let envelope = sample_envelope_with_payload(
        "approval-unwired",
        ProductInboundPayload::ApprovalResolution(
            ApprovalResolutionPayload::new(gate_ref.as_str(), ApprovalDecision::ApproveOnce)
                .expect("approval payload"),
        ),
    );

    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("unwired approval service");

    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::Unavailable,
            status_code: 503,
            retryable: true,
            ..
        }
    ));
}

#[tokio::test]
async fn before_inbound_policy_rewrite_reaches_inbound_turn_service() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.rewrite_user_message(
        UserMessagePayload::new(
            "rewritten by policy",
            vec![],
            ProductTriggerReason::DirectChat,
        )
        .expect("valid rewrite"),
    );
    let envelope = sample_envelope("policy-rewrite");

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));
    assert_eq!(policy.request_count(), 1);
    let request = &policy.requests()[0];
    assert_eq!(request.adapter_id.as_str(), "test_adapter");
    assert_eq!(request.installation_id.as_str(), "install_alpha");
    assert_eq!(request.user_message.text, "hello");
    assert_eq!(request.external_actor_ref.id(), "user1");
    assert_eq!(
        request.external_conversation_ref.conversation_fingerprint(),
        "space:0:;conversation:5:conv1;topic:0:;"
    );
    assert_eq!(
        request.source_binding_key.as_str(),
        "space:0:;conversation:5:conv1;topic:0:;"
    );
    assert_eq!(
        request.rate_limit_key.as_str(),
        "space:0:;conversation:5:conv1;topic:0:;"
    );
    let accepted = inbound.accepted_envelopes();
    assert_eq!(accepted.len(), 1);
    let ProductInboundPayload::UserMessage(payload) = accepted[0].payload() else {
        panic!("expected rewritten user message payload")
    };
    assert_eq!(payload.text, "rewritten by policy");
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn before_inbound_policy_path_probes_replay_once() {
    let inbound = Arc::new(ReplayCountingInboundTurnService::default());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let policy = Arc::new(FakeBeforeInboundPolicy::new());
    policy.rewrite_user_message(
        UserMessagePayload::new("rewritten once", vec![], ProductTriggerReason::DirectChat)
            .expect("valid rewrite"),
    );
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone(), binding)
        .with_before_inbound_policy(policy.clone());
    let envelope = sample_envelope("policy-replay-once");

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));
    assert_eq!(inbound.replay_attempt_count(), 1);
    assert_eq!(inbound.attempt_count(), 1);
    let accepted = inbound.accepted_envelopes();
    let ProductInboundPayload::UserMessage(payload) = accepted[0].payload() else {
        panic!("expected rewritten user message payload")
    };
    assert_eq!(payload.text, "rewritten once");
    assert_eq!(policy.request_count(), 1);
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn before_inbound_policy_rewrite_revalidates_payload_before_turn_path() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.rewrite_user_message(UserMessagePayload {
        text: "a".repeat(64 * 1024 + 1),
        attachments: vec![],
        trigger: ProductTriggerReason::DirectChat,
    });
    let envelope = sample_envelope("policy-rewrite-invalid");

    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("invalid policy rewrite should fail before staging");

    assert!(!err.is_retryable());
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);
}

#[tokio::test]
async fn before_inbound_policy_rejection_skips_transcript_and_turn_path() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.reject(ProductRejection::permanent(
        ProductRejectionKind::PolicyDenied,
        "blocked by before-inbound policy",
    ));
    let envelope = sample_envelope("policy-reject");

    let ack = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("policy rejection ack");

    let ProductInboundAck::Rejected(rejection) = ack else {
        panic!("expected rejected ack")
    };
    assert_eq!(rejection.kind, ProductRejectionKind::PolicyDenied);
    assert_eq!(
        rejection.disposition(),
        ProductRejectionDisposition::Permanent
    );
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 1);
    let actions = ledger.settled_actions();
    assert_eq!(
        actions[0].dispatch_kind,
        Some(ActionDispatchKind::Rejected {
            kind: ProductRejectionKind::PolicyDenied
        })
    );

    let replay = workflow
        .accept_inbound(envelope)
        .await
        .expect("policy rejection replay");
    assert!(matches!(replay, ProductInboundAck::Duplicate { .. }));
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
}

#[tokio::test]
async fn before_inbound_policy_retryable_rejection_releases_fingerprint() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.reject(ProductRejection::retryable(
        ProductRejectionKind::PolicyDenied,
        "transient policy refusal",
    ));
    let envelope = sample_envelope("policy-reject-retryable");

    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("retryable rejection still returns an ack");
    let ProductInboundAck::Rejected(rejection) = first else {
        panic!("expected rejected ack")
    };
    assert_eq!(
        rejection.disposition(),
        ProductRejectionDisposition::Retryable
    );
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);
    assert_eq!(ledger.released_count(), 1);
    assert!(
        ledger
            .last_released_action()
            .expect("released action")
            .dispatch_kind
            .is_none()
    );

    // Re-submitting the same envelope must re-invoke the policy (no duplicate
    // replay caching), because retryable rejections release the fingerprint.
    let second = workflow
        .accept_inbound(envelope)
        .await
        .expect("released fingerprint should let retryable rejection re-run policy");
    assert!(matches!(second, ProductInboundAck::Rejected(_)));
    assert_eq!(policy.request_count(), 2);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
}

#[tokio::test]
async fn before_inbound_policy_rewrite_replays_rewritten_outcome_on_duplicate() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.rewrite_user_message(
        UserMessagePayload::new(
            "rewritten by policy",
            vec![],
            ProductTriggerReason::DirectChat,
        )
        .expect("valid rewrite"),
    );
    let envelope = sample_envelope("policy-rewrite-dup");

    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("first accept");
    let ProductInboundAck::Accepted {
        submitted_run_id: first_run,
        ..
    } = first
    else {
        panic!("expected accepted ack on first dispatch")
    };
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 1);
    assert_eq!(ledger.settled_count(), 1);

    let replay = workflow
        .accept_inbound(envelope)
        .await
        .expect("duplicate replay");
    let ProductInboundAck::Duplicate { prior } = replay else {
        panic!("expected duplicate ack on replay")
    };
    let ProductInboundAck::Accepted {
        submitted_run_id: prior_run,
        ..
    } = *prior
    else {
        panic!("expected replayed prior accepted ack")
    };
    assert_eq!(prior_run, first_run);
    // Policy and inbound must NOT be re-invoked on duplicate replay.
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 1);
}

#[tokio::test]
async fn before_inbound_policy_does_not_block_staged_deferred_retry() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.allow();
    let accepted_message_ref = AcceptedMessageRef::new("msg:policy-busy").expect("valid msg ref");
    let busy_run = TurnRunId::new();
    inbound.program_outcome(InboundTurnOutcome::DeferredBusy {
        accepted_message_ref: accepted_message_ref.clone(),
        active_run_id: busy_run,
        binding: fake_binding(),
    });
    let envelope = sample_envelope("policy-busy-retry");

    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("busy ack");
    assert!(matches!(first, ProductInboundAck::DeferredBusy { .. }));
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.attempt_count(), 1);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);

    let submitted_run_id = TurnRunId::new();
    inbound.program_replay_outcome(InboundTurnOutcome::Submitted {
        accepted_message_ref,
        submitted_run_id,
        binding: fake_binding(),
    });
    policy.reject(ProductRejection::permanent(
        ProductRejectionKind::PolicyDenied,
        "policy changed after message was staged",
    ));

    let second = workflow
        .accept_inbound(envelope)
        .await
        .expect("staged message replay should bypass policy");
    assert!(matches!(
        second,
        ProductInboundAck::Accepted {
            submitted_run_id: run_id,
            ..
        } if run_id == submitted_run_id
    ));
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.replay_attempt_count(), 2);
    assert_eq!(inbound.attempt_count(), 1);
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn before_inbound_policy_transient_failure_releases_fingerprint() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.force_failure(ProductWorkflowError::Transient {
        reason: "policy store unavailable".into(),
    });
    let envelope = sample_envelope("policy-transient");

    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("policy failure should be retryable");
    assert!(first.is_retryable());
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);

    let second = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("released fingerprint should retry policy");
    assert!(second.is_retryable());
    assert_eq!(policy.request_count(), 2);
    assert_eq!(inbound.accepted_count(), 0);
}

#[tokio::test]
async fn before_inbound_policy_retryable_failure_releases_fingerprint() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.force_failure(ProductWorkflowError::BeforeInboundPolicyFailed {
        reason: "policy cache miss".into(),
        permanent: false,
    });
    let envelope = sample_envelope("policy-retryable-failure");

    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("retryable policy failure should release fingerprint");
    assert!(first.is_retryable());
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);

    let second = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("released fingerprint should retry policy");
    assert!(second.is_retryable());
    assert_eq!(policy.request_count(), 2);
    assert_eq!(inbound.accepted_count(), 0);
}

#[tokio::test]
async fn before_inbound_policy_timeout_releases_fingerprint_for_retry() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.delay_responses_by(StdDuration::from_millis(200));
    let envelope = sample_envelope("policy-timeout-release");

    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("timed-out policy should be retryable");
    assert!(first.is_retryable());
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);
    assert_eq!(ledger.released_count(), 1);
    assert!(
        ledger
            .last_released_action()
            .expect("released action")
            .dispatch_kind
            .is_none()
    );

    let second = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("released fingerprint should retry timed-out policy");
    assert!(second.is_retryable());
    assert_eq!(policy.request_count(), 2);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);
    assert_eq!(ledger.released_count(), 2);
}

#[tokio::test]
async fn before_inbound_policy_permanent_failure_settles_terminal_rejection() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.force_failure(ProductWorkflowError::BeforeInboundPolicyFailed {
        reason: "policy configuration is invalid".into(),
        permanent: true,
    });
    let envelope = sample_envelope("policy-permanent-failure");

    let err = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("permanent policy failure should surface rejected error");
    assert!(!err.is_retryable());
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 1);
    let actions = ledger.settled_actions();
    assert_eq!(
        actions[0].dispatch_kind,
        Some(ActionDispatchKind::Rejected {
            kind: ProductRejectionKind::PolicyDenied
        })
    );

    let replay = workflow
        .accept_inbound(envelope)
        .await
        .expect("terminal policy failure should replay duplicate ack");
    let ProductInboundAck::Duplicate { prior } = replay else {
        panic!("expected duplicate replay")
    };
    let ProductInboundAck::Rejected(rejection) = *prior else {
        panic!("expected rejected prior outcome")
    };
    assert_eq!(rejection.kind, ProductRejectionKind::PolicyDenied);
    assert_eq!(
        rejection.disposition(),
        ProductRejectionDisposition::Permanent
    );
    let rejection_debug = format!("{rejection:?}");
    assert!(
        !rejection_debug.contains("policy configuration is invalid"),
        "durable rejection ack must not expose raw policy internals: {rejection_debug}"
    );
    assert!(
        rejection_debug.contains("<redacted>"),
        "durable rejection reason should remain redacted: {rejection_debug}"
    );
}

#[tokio::test]
async fn fake_before_inbound_policy_uses_programmed_outcomes_in_order() {
    let policy = FakeBeforeInboundPolicy::new();
    let envelope = sample_envelope("fake-policy-sequence");
    let ProductInboundPayload::UserMessage(payload) = envelope.payload() else {
        panic!("expected user message")
    };
    policy.program_outcomes([
        Ok(BeforeInboundPolicyOutcome::RewriteUserMessage(
            UserMessagePayload::new("first", vec![], ProductTriggerReason::DirectChat)
                .expect("valid rewrite"),
        )),
        Ok(BeforeInboundPolicyOutcome::Reject(
            ProductRejection::retryable(ProductRejectionKind::PolicyDenied, "try later"),
        )),
    ]);
    policy.allow();

    let first = policy
        .check_user_message(BeforeInboundPolicyRequest::new(&envelope, payload).expect("request"))
        .await
        .expect("first policy result");
    assert!(matches!(
        first,
        BeforeInboundPolicyOutcome::RewriteUserMessage(rewritten) if rewritten.text == "first"
    ));

    let second = policy
        .check_user_message(BeforeInboundPolicyRequest::new(&envelope, payload).expect("request"))
        .await
        .expect("second policy result");
    assert!(matches!(second, BeforeInboundPolicyOutcome::Reject(_)));

    let third = policy
        .check_user_message(BeforeInboundPolicyRequest::new(&envelope, payload).expect("request"))
        .await
        .expect("fallback policy result");
    assert_eq!(third, BeforeInboundPolicyOutcome::Allow);
}

#[tokio::test]
async fn fake_inbound_turn_service_replays_programmed_outcomes_in_order() {
    let inbound = FakeInboundTurnService::new();
    let envelope = sample_envelope("fake-replay-sequence");
    let first_run = TurnRunId::new();
    let second_run = TurnRunId::new();
    inbound.program_replay_outcomes([
        InboundTurnOutcome::DeferredBusy {
            accepted_message_ref: AcceptedMessageRef::new("msg:first").expect("valid"),
            active_run_id: first_run,
            binding: fake_binding(),
        },
        InboundTurnOutcome::Submitted {
            accepted_message_ref: AcceptedMessageRef::new("msg:second").expect("valid"),
            submitted_run_id: second_run,
            binding: fake_binding(),
        },
    ]);

    let first = inbound
        .replay_accepted_user_message(&envelope)
        .await
        .expect("first replay")
        .expect("first programmed outcome");
    assert!(matches!(
        first,
        InboundTurnOutcome::DeferredBusy { active_run_id, .. } if active_run_id == first_run
    ));
    let second = inbound
        .replay_accepted_user_message(&envelope)
        .await
        .expect("second replay")
        .expect("second programmed outcome");
    assert!(matches!(
        second,
        InboundTurnOutcome::Submitted { submitted_run_id, .. } if submitted_run_id == second_run
    ));
    assert!(
        inbound
            .replay_accepted_user_message(&envelope)
            .await
            .expect("third replay")
            .is_none()
    );
}

#[tokio::test]
async fn noop_returns_noop_ack() {
    let (workflow, inbound, ledger) = build_workflow();
    let envelope = sample_noop_envelope("noop1");

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert!(matches!(ack, ProductInboundAck::NoOp));
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn typed_cancel_control_action_uses_submit_door_without_command_text() {
    let (workflow, inbound, ledger) = build_workflow();
    let envelope = sample_envelope_with_payload(
        "typed-cancel-control",
        ProductInboundPayload::ControlAction(ProductControlActionPayload::CancelRun {
            run_id: TurnRunId::new(),
        }),
    );

    let ack = workflow
        .submit_inbound(envelope)
        .await
        .expect("typed control action returns product-safe ack");

    assert!(matches!(
        ack,
        ProductInboundAck::Rejected(ProductRejection {
            kind: ProductRejectionKind::InvalidRequest,
            disposition: ProductRejectionDisposition::Permanent,
            ..
        })
    ));
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn subscription_request_via_accept_inbound_rejects_before_mutating_ledger() {
    let (workflow, inbound, ledger, binding_service) = build_workflow_with_binding();
    let envelope = sample_envelope_with_payload(
        "projection-wrong-entrypoint",
        ProductInboundPayload::SubscriptionRequest(
            ProjectionSubscriptionPayload::new(None, None).expect("valid subscription"),
        ),
    );

    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("subscription requests use the projection resolver, not accept_inbound");

    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::InvalidRequest,
            status_code: 400,
            retryable: false,
            ..
        }
    ));
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(binding_service.resolve_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);
    assert_eq!(ledger.released_count(), 0);
}

#[tokio::test]
async fn subscription_request_via_submit_inbound_rejects_before_mutating_ledger() {
    let (workflow, inbound, ledger, binding_service) = build_workflow_with_binding();
    let envelope = sample_envelope_with_payload(
        "projection-subscribe-wrong-submit-door",
        ProductInboundPayload::SubscriptionRequest(
            ProjectionSubscriptionPayload::new(None, None).expect("valid subscription"),
        ),
    );

    let err = workflow.submit_inbound(envelope).await.expect_err(
        "projection subscriptions use the subscribe projection door, not submit_inbound",
    );

    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::InvalidRequest,
            status_code: 400,
            retryable: false,
            ..
        }
    ));
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(binding_service.resolve_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);
    assert_eq!(ledger.released_count(), 0);
}

#[tokio::test]
async fn projection_read_via_submit_inbound_rejects_before_mutating_ledger() {
    let (workflow, inbound, ledger, binding_service) = build_workflow_with_binding();
    let envelope = sample_envelope_with_payload(
        "projection-read-wrong-entrypoint",
        ProductInboundPayload::ProjectionRead(
            ProjectionReadPayload::new(None, None, Some(25)).expect("valid read"),
        ),
    );

    let err = workflow
        .submit_inbound(envelope)
        .await
        .expect_err("projection reads use the read projection door, not submit_inbound");

    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::InvalidRequest,
            status_code: 400,
            retryable: false,
            ..
        }
    ));
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(binding_service.resolve_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);
    assert_eq!(ledger.released_count(), 0);
}

#[tokio::test]
async fn projection_read_resolves_external_refs_through_read_door() {
    let (workflow, inbound, ledger, binding_service) = build_workflow_with_binding();
    let binding = fake_binding();
    let cursor = ProjectionCursor::new("cursor:projection-read-1").expect("valid cursor");
    let envelope = sample_envelope_with_payload(
        "projection-read-1",
        ProductInboundPayload::ProjectionRead(
            ProjectionReadPayload::new(
                Some(binding.thread_id.as_str().to_string()),
                Some(cursor.clone()),
                Some(50),
            )
            .expect("valid read"),
        ),
    );
    binding_service.program_binding(envelope.source_binding_key(), binding.clone());
    let input = ProductProjectionReadInput::from_inbound_envelope(&envelope).expect("read input");

    let read = workflow
        .read_projection(input)
        .await
        .expect("projection read");

    assert_eq!(read.actor.user_id, binding.actor_user_id);
    assert_eq!(read.scope.tenant_id, binding.tenant_id);
    assert_eq!(read.scope.agent_id, binding.agent_id);
    assert_eq!(read.scope.project_id, binding.project_id);
    assert_eq!(read.scope.thread_id, binding.thread_id);
    assert_eq!(read.after_cursor, Some(cursor));
    assert_eq!(read.limit, Some(50));
    assert_eq!(binding_service.resolve_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);
    assert_eq!(ledger.released_count(), 0);
}

#[tokio::test]
async fn projection_read_accepts_canonical_subject_without_inbound_envelope() {
    let (workflow, inbound, ledger, binding_service) = build_workflow_with_binding();
    let binding = fake_binding();
    let actor = TurnActor::new(binding.actor_user_id.clone());
    let scope = TurnScope::new(
        binding.tenant_id.clone(),
        binding.agent_id.clone(),
        binding.project_id.clone(),
        binding.thread_id.clone(),
    );
    let cursor = ProjectionCursor::new("cursor:canonical-read").expect("valid cursor");

    let read = workflow
        .read_projection(ProductProjectionReadInput::new(
            ProductProjectionSubject::canonical(actor.clone(), scope.clone()),
            Some(binding.thread_id.as_str().to_string()),
            Some(cursor.clone()),
            Some(10),
        ))
        .await
        .expect("canonical projection read");

    assert_eq!(read.actor, actor);
    assert_eq!(read.scope, scope);
    assert_eq!(read.after_cursor, Some(cursor));
    assert_eq!(read.limit, Some(10));
    assert_eq!(binding_service.resolve_count(), 0);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);
    assert_eq!(ledger.released_count(), 0);
}

#[tokio::test]
async fn projection_subscription_resolves_through_binding_service() {
    let (workflow, inbound, ledger, binding_service) = build_workflow_with_binding();
    let binding = fake_binding();
    let cursor = ProjectionCursor::new("cursor:projection-1").expect("valid cursor");
    let envelope = sample_envelope_with_payload(
        "projection-1",
        ProductInboundPayload::SubscriptionRequest(
            ProjectionSubscriptionPayload::new(
                Some(binding.thread_id.as_str().to_string()),
                Some(cursor.clone()),
            )
            .expect("valid subscription"),
        ),
    );
    binding_service.program_binding(envelope.source_binding_key(), binding.clone());

    let input =
        ProductProjectionSubscribeInput::from_inbound_envelope(&envelope).expect("subscribe input");
    let subscription = workflow
        .subscribe_projection(input)
        .await
        .expect("projection subscription");

    assert_eq!(subscription.actor.user_id, binding.actor_user_id);
    assert_eq!(subscription.scope.tenant_id, binding.tenant_id);
    assert_eq!(subscription.scope.agent_id, binding.agent_id);
    assert_eq!(subscription.scope.project_id, binding.project_id);
    assert_eq!(subscription.scope.thread_id, binding.thread_id);
    assert_eq!(subscription.after_cursor, Some(cursor));
    assert_eq!(binding_service.resolve_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);
    assert_eq!(ledger.released_count(), 0);
}

#[tokio::test]
async fn projection_subscription_accepts_canonical_subject_without_inbound_envelope() {
    let (workflow, inbound, ledger, binding_service) = build_workflow_with_binding();
    let binding = fake_binding();
    let actor = TurnActor::new(binding.actor_user_id.clone());
    let scope = TurnScope::new(
        binding.tenant_id.clone(),
        binding.agent_id.clone(),
        binding.project_id.clone(),
        binding.thread_id.clone(),
    );
    let cursor = ProjectionCursor::new("cursor:canonical-subscribe").expect("valid cursor");

    let subscription = workflow
        .subscribe_projection(ProductProjectionSubscribeInput::new(
            ProductProjectionSubject::canonical(actor.clone(), scope.clone()),
            Some(binding.thread_id.as_str().to_string()),
            Some(cursor.clone()),
        ))
        .await
        .expect("canonical projection subscription");

    assert_eq!(subscription.actor, actor);
    assert_eq!(subscription.scope, scope);
    assert_eq!(subscription.after_cursor, Some(cursor));
    assert_eq!(binding_service.resolve_count(), 0);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);
    assert_eq!(ledger.released_count(), 0);
}

#[tokio::test]
async fn projection_subscription_rejects_non_subscription_payload() {
    let (workflow, _inbound, _ledger, _binding_service) = build_workflow_with_binding();

    let err = workflow
        .resolve_projection_subscription(sample_envelope("projection-non-subscription"))
        .await
        .expect_err("non-subscription payload rejects");

    assert!(matches!(
        err,
        ProductAdapterError::MalformedInboundPayload { .. }
    ));
}

#[tokio::test]
async fn projection_subscription_rejects_mismatched_thread_hint() {
    let (workflow, _inbound, _ledger, binding_service) = build_workflow_with_binding();
    let binding = fake_binding();
    let envelope = sample_envelope_with_payload(
        "projection-mismatch",
        ProductInboundPayload::SubscriptionRequest(
            ProjectionSubscriptionPayload::new(Some("thread:other".into()), None)
                .expect("valid subscription"),
        ),
    );
    binding_service.program_binding(envelope.source_binding_key(), binding);

    let err = workflow
        .resolve_projection_subscription(envelope)
        .await
        .expect_err("mismatched hint rejects");

    match err {
        ProductAdapterError::WorkflowRejected {
            kind,
            status_code,
            retryable,
            ..
        } => {
            assert_eq!(kind, ProductWorkflowRejectionKind::InvalidRequest);
            assert_eq!(status_code, 400);
            assert!(!retryable);
        }
        other => panic!("expected workflow rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn projection_subscription_rejects_malformed_thread_hint() {
    let (workflow, _inbound, _ledger, binding_service) = build_workflow_with_binding();
    let binding = fake_binding();
    let envelope = sample_envelope_with_payload(
        "projection-malformed-hint",
        ProductInboundPayload::SubscriptionRequest(
            ProjectionSubscriptionPayload::new(Some("thread/invalid".into()), None)
                .expect("adapter accepts opaque hint"),
        ),
    );
    binding_service.program_binding(envelope.source_binding_key(), binding);

    let err = workflow
        .resolve_projection_subscription(envelope)
        .await
        .expect_err("malformed hint rejects");

    assert!(matches!(
        err,
        ProductAdapterError::MalformedInboundPayload { .. }
    ));
}

#[tokio::test]
async fn projection_subscription_requires_existing_conversation_binding() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            TenantId::new("tenant:alpha").expect("tenant"),
            ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter"),
            ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install"),
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    let binding = product_binding_service(
        conversations,
        vec![(
            "test_adapter",
            "install_alpha",
            "tenant:alpha",
            "agent:alpha",
            Some("project:alpha"),
        )],
    );
    let workflow = DefaultProductWorkflow::new(
        Arc::new(FakeInboundTurnService::new()),
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );
    let envelope = sample_envelope_with_payload(
        "projection-missing-binding",
        ProductInboundPayload::SubscriptionRequest(
            ProjectionSubscriptionPayload::new(None, None).expect("valid subscription"),
        ),
    );

    let err = workflow
        .resolve_projection_subscription(envelope)
        .await
        .expect_err("subscription must not create a missing binding");

    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::ScopeNotFound,
            status_code: 404,
            retryable: false,
            ..
        }
    ));
}

#[tokio::test]
async fn preconfigured_actor_binding_accepts_user_message_without_legacy_pairing() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    let binding =
        product_binding_service_with_preconfigured_actor(conversations, "user:preconfigured-slack");
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );

    let ack = workflow
        .accept_inbound(sample_envelope("preconfigured-actor"))
        .await
        .expect("preconfigured actor should be accepted");

    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));
    let submission = coordinator
        .submissions()
        .into_iter()
        .next()
        .expect("turn should be submitted");
    assert_eq!(
        submission.actor.user_id.as_str(),
        "user:preconfigured-slack"
    );
}

#[tokio::test]
async fn preconfigured_actor_binding_rejects_unconfigured_actor() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    let conversation_port: Arc<dyn ironclaw_conversations::ConversationBindingService> =
        conversations.clone();
    let actor_pairings: Arc<dyn ironclaw_conversations::ConversationActorPairingService> =
        conversations;
    let scope = ProductInstallationScope::with_default_scope(
        TenantId::new("tenant:alpha").expect("tenant"),
        AgentId::new("agent:alpha").expect("agent"),
        Some(ProjectId::new("project:alpha").expect("project")),
    )
    .with_preconfigured_actor_binding(
        ExternalActorRef::new("test", "different-user", None::<String>).expect("actor"),
        UserId::new("user:alice").expect("user"),
        actor_pairings,
    );
    let resolver = StaticProductInstallationResolver::new([(
        ProductInstallationKey::new(
            ProductAdapterId::new("test_adapter").expect("adapter"),
            AdapterInstallationId::new("install_alpha").expect("installation"),
        ),
        scope,
    )]);
    let binding = ProductConversationBindingService::new(conversation_port.clone(), resolver);
    let workflow = DefaultProductWorkflow::new(
        Arc::new(DefaultInboundTurnService::new(
            binding.clone(),
            InMemorySessionThreadService::default(),
            Arc::new(RecordingTurnCoordinator::default()),
        )),
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );

    let err = workflow
        .accept_inbound(sample_envelope("unconfigured-actor"))
        .await
        .expect_err("unconfigured actor should fail closed");

    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::ScopeNotFound,
            status_code: 404,
            retryable: false,
            ..
        }
    ));
}

#[tokio::test]
async fn actor_user_resolver_accepts_user_message_without_legacy_pairing() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    let (binding, actor_resolver) = product_binding_service_with_actor_user_resolver(
        conversations,
        [(
            ExternalActorRef::new("test", "user1", None::<String>).expect("actor"),
            UserId::new("user:resolved-slack").expect("user"),
        )],
    );
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );

    let ack = workflow
        .accept_inbound(sample_envelope("resolver-actor"))
        .await
        .expect("resolved actor should be accepted");

    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));
    let submission = coordinator
        .submissions()
        .into_iter()
        .next()
        .expect("turn should be submitted");
    assert_eq!(submission.actor.user_id.as_str(), "user:resolved-slack");
    assert_eq!(actor_resolver.calls().len(), 1);
}

#[tokio::test]
async fn actor_user_resolver_rejects_unknown_actor_before_turn_submission() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    let (binding, actor_resolver) = product_binding_service_with_actor_user_resolver(
        conversations,
        std::iter::empty::<(ExternalActorRef, UserId)>(),
    );
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let workflow = DefaultProductWorkflow::new(
        Arc::new(DefaultInboundTurnService::new(
            binding.clone(),
            InMemorySessionThreadService::default(),
            coordinator.clone(),
        )),
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );

    let err = workflow
        .accept_inbound(sample_envelope("resolver-missing-actor"))
        .await
        .expect_err("unknown actor should require binding");

    assert!(coordinator.submissions().is_empty());
    assert_eq!(actor_resolver.calls().len(), 1);
    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::ScopeNotFound,
            status_code: 404,
            retryable: false,
            ..
        }
    ));
}

#[tokio::test]
async fn actor_user_resolver_propagates_resolver_error_without_turn_submission() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    let binding = product_binding_service_with_actor_user_resolver_arc(
        conversations,
        Arc::new(FailingProductActorUserResolver),
    );
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let workflow = DefaultProductWorkflow::new(
        Arc::new(DefaultInboundTurnService::new(
            binding.clone(),
            InMemorySessionThreadService::default(),
            coordinator.clone(),
        )),
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );

    let err = workflow
        .accept_inbound(sample_envelope("resolver-error"))
        .await
        .expect_err("resolver error should fail the workflow");

    assert!(coordinator.submissions().is_empty());
    assert!(matches!(err, ProductAdapterError::Internal { .. }));
}

#[tokio::test]
async fn lookup_binding_with_actor_user_resolver_uses_existing_pairings_only() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    let (binding, actor_resolver) = product_binding_service_with_actor_user_resolver(
        conversations,
        std::iter::empty::<(ExternalActorRef, UserId)>(),
    );

    let err = binding
        .lookup_binding(ResolveBindingRequest::from_envelope(&sample_envelope(
            "lookup-resolver-missing-actor",
        )))
        .await
        .expect_err("lookup must require an existing durable actor pairing");

    assert!(
        actor_resolver.calls().is_empty(),
        "existing-only lookup must not trigger resolver pairing challenges"
    );
    assert!(matches!(err, ProductWorkflowError::BindingRequired { .. }));
}

#[tokio::test]
async fn lookup_binding_with_actor_user_resolver_ignores_resolver_failures() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    let binding = product_binding_service_with_actor_user_resolver_arc(
        conversations,
        Arc::new(FailingProductActorUserResolver),
    );

    let err = binding
        .lookup_binding(ResolveBindingRequest::from_envelope(&sample_envelope(
            "lookup-resolver-error",
        )))
        .await
        .expect_err("lookup should fail from missing durable pairing, not resolver backend");

    assert!(matches!(err, ProductWorkflowError::BindingRequired { .. }));
}

#[tokio::test]
async fn lookup_binding_with_actor_user_resolver_returns_existing_actor_pairing() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            TenantId::new("tenant:alpha").expect("tenant"),
            ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter"),
            ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install"),
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:paired-bob").expect("user"),
        )
        .await;
    let seed_binding = product_binding_service(
        conversations.clone(),
        vec![(
            "test_adapter",
            "install_alpha",
            "tenant:alpha",
            "agent:alpha",
            Some("project:alpha"),
        )],
    );
    let envelope = sample_envelope("lookup-resolver-mismatch");
    seed_binding
        .resolve_binding(ResolveBindingRequest::from_envelope(&envelope))
        .await
        .expect("seed canonical conversation binding");
    let (binding, actor_resolver) = product_binding_service_with_actor_user_resolver(
        conversations,
        [(
            ExternalActorRef::new("test", "user1", None::<String>).expect("actor"),
            UserId::new("user:resolved-alice").expect("user"),
        )],
    );

    let resolved = binding
        .lookup_binding(ResolveBindingRequest::from_envelope(&envelope))
        .await
        .expect("lookup should use the existing durable actor pairing");

    assert!(
        actor_resolver.calls().is_empty(),
        "existing-only lookup must not reinterpret durable pairing through resolver"
    );
    assert_eq!(resolved.actor_user_id.as_str(), "user:paired-bob");
}

#[tokio::test]
async fn concrete_product_workflow_accepts_user_message_for_trusted_installation() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            TenantId::new("tenant:alpha").expect("tenant"),
            ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter"),
            ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install"),
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    let binding = product_binding_service(
        conversations,
        vec![(
            "test_adapter",
            "install_alpha",
            "tenant:alpha",
            "agent:alpha",
            Some("project:alpha"),
        )],
    );
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );
    let envelope = sample_envelope("concrete-happy");

    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("accepted");
    let duplicate = workflow
        .accept_inbound(envelope)
        .await
        .expect("duplicate replay");

    assert!(matches!(first, ProductInboundAck::Accepted { .. }));
    assert!(matches!(duplicate, ProductInboundAck::Duplicate { .. }));
    let submissions = coordinator.submissions();
    assert_eq!(submissions.len(), 1);
    assert_eq!(submissions[0].scope.tenant_id.as_str(), "tenant:alpha");
    assert_eq!(
        submissions[0].scope.agent_id.as_ref().map(AgentId::as_str),
        Some("agent:alpha")
    );
    assert_eq!(
        submissions[0]
            .scope
            .project_id
            .as_ref()
            .map(ProjectId::as_str),
        Some("project:alpha")
    );
    assert_eq!(submissions[0].actor.user_id.as_str(), "user:alice");
}

#[tokio::test]
async fn concrete_product_workflow_accepts_shared_route_participant_on_existing_thread() {
    let tenant_id = TenantId::new("tenant:alpha").expect("tenant");
    let adapter_kind = ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter");
    let installation_id =
        ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install");
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            tenant_id.clone(),
            adapter_kind.clone(),
            installation_id.clone(),
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    conversations
        .pair_external_actor(
            tenant_id.clone(),
            adapter_kind,
            installation_id,
            ironclaw_conversations::ExternalActorRef::new("test", "user2").expect("actor"),
            UserId::new("user:bob").expect("user"),
        )
        .await;
    let binding = product_binding_service(
        conversations.clone(),
        vec![(
            "test_adapter",
            "install_alpha",
            "tenant:alpha",
            "agent:alpha",
            Some("project:alpha"),
        )],
    );
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );

    workflow
        .accept_inbound(sample_envelope_with_payload(
            "shared-alice",
            ProductInboundPayload::UserMessage(
                UserMessagePayload::new("hello shared", vec![], ProductTriggerReason::BotMention)
                    .expect("message"),
            ),
        ))
        .await
        .expect("alice shared message accepted");
    let shared_thread_id = coordinator.submissions()[0].scope.thread_id.clone();
    conversations
        .add_thread_participant(
            &tenant_id,
            &shared_thread_id,
            UserId::new("user:bob").expect("user"),
        )
        .await
        .expect("bob participant added");

    workflow
        .accept_inbound(sample_envelope_with_context(
            ProductAdapterId::new("test_adapter").expect("adapter"),
            AdapterInstallationId::new("install_alpha").expect("install"),
            ExternalEventId::new("evt:shared-bob").expect("event"),
            ExternalActorRef::new("test", "user2", Option::<String>::None).expect("actor"),
            ExternalConversationRef::new(None, "conv1", None, None).expect("conversation"),
            ProductInboundPayload::UserMessage(
                UserMessagePayload::new("hello from bob", vec![], ProductTriggerReason::BotMention)
                    .expect("message"),
            ),
        ))
        .await
        .expect("shared participant accepted on existing thread");

    let submissions = coordinator.submissions();
    assert_eq!(submissions.len(), 2);
    assert_eq!(
        submissions[0].scope.thread_id,
        submissions[1].scope.thread_id
    );
    assert_eq!(submissions[0].actor.user_id.as_str(), "user:alice");
    assert_eq!(submissions[1].actor.user_id.as_str(), "user:bob");
    assert_eq!(
        submissions[0].scope.explicit_owner_user_id(),
        Some(&UserId::new("user:team-agent").expect("team subject"))
    );
    assert_eq!(
        submissions[1].scope.explicit_owner_user_id(),
        Some(&UserId::new("user:team-agent").expect("team subject"))
    );
}

#[tokio::test]
async fn concrete_product_workflow_persists_first_bind_default_scope() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            TenantId::new("tenant:alpha").expect("tenant"),
            ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter"),
            ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install"),
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    let binding_alpha = product_binding_service(
        conversations.clone(),
        vec![(
            "test_adapter",
            "install_alpha",
            "tenant:alpha",
            "agent:alpha",
            Some("project:alpha"),
        )],
    );
    let workflow_alpha = DefaultProductWorkflow::new(
        Arc::new(DefaultInboundTurnService::new(
            binding_alpha.clone(),
            InMemorySessionThreadService::default(),
            Arc::new(RecordingTurnCoordinator::default()),
        )),
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding_alpha),
    );
    workflow_alpha
        .accept_inbound(sample_envelope("persisted-default-scope"))
        .await
        .expect("first bind accepted");

    let binding_beta = product_binding_service(
        conversations,
        vec![(
            "test_adapter",
            "install_alpha",
            "tenant:alpha",
            "agent:beta",
            Some("project:beta"),
        )],
    );
    let workflow_beta = DefaultProductWorkflow::new(
        Arc::new(FakeInboundTurnService::new()),
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding_beta),
    );
    let subscription = workflow_beta
        .resolve_projection_subscription(sample_envelope_with_payload(
            "projection-existing-scope",
            ProductInboundPayload::SubscriptionRequest(
                ProjectionSubscriptionPayload::new(None, None).expect("valid subscription"),
            ),
        ))
        .await
        .expect("existing binding resolves");

    assert_eq!(
        subscription.scope.agent_id.as_ref().map(AgentId::as_str),
        Some("agent:alpha")
    );
    assert_eq!(
        subscription
            .scope
            .project_id
            .as_ref()
            .map(ProjectId::as_str),
        Some("project:alpha")
    );
}

#[tokio::test]
async fn concrete_product_workflow_keeps_installations_tenant_isolated() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    for (install, tenant, user) in [
        ("install_alpha", "tenant:alpha", "user:alice"),
        ("install_beta", "tenant:beta", "user:bob"),
    ] {
        conversations
            .pair_external_actor(
                TenantId::new(tenant).expect("tenant"),
                ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter"),
                ironclaw_conversations::AdapterInstallationId::new(install).expect("install"),
                ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
                UserId::new(user).expect("user"),
            )
            .await;
    }
    let binding = product_binding_service(
        conversations,
        vec![
            (
                "test_adapter",
                "install_alpha",
                "tenant:alpha",
                "agent:alpha",
                None,
            ),
            (
                "test_adapter",
                "install_beta",
                "tenant:beta",
                "agent:beta",
                None,
            ),
        ],
    );
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );

    workflow
        .accept_inbound(sample_envelope("tenant-a"))
        .await
        .expect("tenant a accepted");
    workflow
        .accept_inbound(sample_envelope_with_context(
            ProductAdapterId::new("test_adapter").expect("adapter"),
            AdapterInstallationId::new("install_beta").expect("install"),
            ExternalEventId::new("evt:tenant-b").expect("event"),
            ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
            ExternalConversationRef::new(None, "conv1", None, None).expect("conversation"),
            ProductInboundPayload::UserMessage(
                UserMessagePayload::new("hello beta", vec![], ProductTriggerReason::DirectChat)
                    .expect("message"),
            ),
        ))
        .await
        .expect("tenant b accepted");

    let submissions = coordinator.submissions();
    assert_eq!(submissions.len(), 2);
    assert_eq!(submissions[0].scope.tenant_id.as_str(), "tenant:alpha");
    assert_eq!(submissions[0].actor.user_id.as_str(), "user:alice");
    assert_eq!(submissions[1].scope.tenant_id.as_str(), "tenant:beta");
    assert_eq!(submissions[1].actor.user_id.as_str(), "user:bob");
    assert_ne!(
        submissions[0].scope.thread_id,
        submissions[1].scope.thread_id
    );
}

#[tokio::test]
async fn shared_route_without_configured_subject_requires_binding() {
    let tenant_id = TenantId::new("tenant:alpha").expect("tenant");
    let adapter_kind = ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter");
    let installation_id =
        ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install");
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            tenant_id.clone(),
            adapter_kind,
            installation_id,
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    let conversation_port: Arc<dyn ironclaw_conversations::ConversationBindingService> =
        conversations;
    let resolver = StaticProductInstallationResolver::new([(
        ProductInstallationKey::new(
            ProductAdapterId::new("test_adapter").expect("adapter"),
            AdapterInstallationId::new("install_alpha").expect("installation"),
        ),
        ProductInstallationScope::with_default_scope(
            tenant_id,
            AgentId::new("agent:alpha").expect("agent"),
            Some(ProjectId::new("project:alpha").expect("project")),
        ),
    )]);
    let binding = ProductConversationBindingService::new(conversation_port.clone(), resolver);
    let envelope = sample_envelope_with_payload(
        "shared-no-subject",
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello shared", vec![], ProductTriggerReason::BotMention)
                .expect("message"),
        ),
    );

    let error = binding
        .resolve_binding(ResolveBindingRequest::from_envelope(&envelope))
        .await
        .expect_err("shared binding must require an explicit subject user");

    assert!(matches!(
        error,
        ProductWorkflowError::BindingRequired { reason }
            if reason == "shared product route requires a configured subject user"
    ));
}

#[tokio::test]
async fn shared_route_uses_conversation_specific_subject_over_installation_default() {
    let tenant_id = TenantId::new("tenant:alpha").expect("tenant");
    let adapter_kind = ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter");
    let installation_id =
        ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install");
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            tenant_id.clone(),
            adapter_kind,
            installation_id,
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    let conversation_port: Arc<dyn ironclaw_conversations::ConversationBindingService> =
        conversations;
    let scope = ProductInstallationScope::with_default_scope(
        tenant_id,
        AgentId::new("agent:alpha").expect("agent"),
        Some(ProjectId::new("project:alpha").expect("project")),
    )
    .with_default_subject_user_id(UserId::new("user:default-team").expect("default subject"))
    .with_conversation_subject_route(
        ProductConversationRouteKey::new(Some("T-team".to_string()), "C-eng".to_string())
            .expect("route key"),
        UserId::new("user:eng-team").expect("route subject"),
    );
    let resolver = StaticProductInstallationResolver::new([(
        ProductInstallationKey::new(
            ProductAdapterId::new("test_adapter").expect("adapter"),
            AdapterInstallationId::new("install_alpha").expect("installation"),
        ),
        scope,
    )]);
    let binding = ProductConversationBindingService::new(conversation_port.clone(), resolver);
    let envelope = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("installation"),
        ExternalEventId::new("evt:shared-route-subject").expect("event"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(Some("T-team"), "C-eng", Some("thread-1"), Some("msg-1"))
            .expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello shared", vec![], ProductTriggerReason::BotMention)
                .expect("message"),
        ),
    );

    let resolved = binding
        .resolve_binding(ResolveBindingRequest::from_envelope(&envelope))
        .await
        .expect("shared binding should resolve");

    assert_eq!(resolved.actor_user_id.as_str(), "user:alice");
    assert_eq!(
        resolved.subject_user_id.as_ref().map(UserId::as_str),
        Some("user:eng-team")
    );
}

#[tokio::test]
async fn static_shared_route_does_not_probe_existing_binding_before_resolve() {
    let tenant_id = TenantId::new("tenant:alpha").expect("tenant");
    let adapter_kind = ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter");
    let installation_id =
        ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install");
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            tenant_id.clone(),
            adapter_kind,
            installation_id,
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    let counted_conversations = Arc::new(CountingConversationBindingService::new(conversations));
    let conversation_port: Arc<dyn ironclaw_conversations::ConversationBindingService> =
        counted_conversations.clone();
    let scope = ProductInstallationScope::with_default_scope(
        tenant_id,
        AgentId::new("agent:alpha").expect("agent"),
        Some(ProjectId::new("project:alpha").expect("project")),
    )
    .with_conversation_subject_route(
        ProductConversationRouteKey::new(Some("T-team".to_string()), "C-eng".to_string())
            .expect("route key"),
        UserId::new("user:eng-team").expect("route subject"),
    );
    let resolver = StaticProductInstallationResolver::new([(
        ProductInstallationKey::new(
            ProductAdapterId::new("test_adapter").expect("adapter"),
            AdapterInstallationId::new("install_alpha").expect("installation"),
        ),
        scope,
    )]);
    let binding = ProductConversationBindingService::new(conversation_port, resolver);
    let envelope = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("installation"),
        ExternalEventId::new("evt:static-shared-no-lookup").expect("event"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(Some("T-team"), "C-eng", Some("thread-1"), Some("msg-1"))
            .expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello shared", vec![], ProductTriggerReason::BotMention)
                .expect("message"),
        ),
    );

    let resolved = binding
        .resolve_binding(ResolveBindingRequest::from_envelope(&envelope))
        .await
        .expect("static shared binding should resolve");

    assert_eq!(
        resolved.subject_user_id.as_ref().map(UserId::as_str),
        Some("user:eng-team")
    );
    assert_eq!(counted_conversations.lookup_count(), 0);
    assert_eq!(counted_conversations.trusted_resolve_count(), 1);
}

#[tokio::test]
async fn shared_route_uses_dynamic_subject_route_resolver_without_rebuilding_scope() {
    let tenant_id = TenantId::new("tenant:alpha").expect("tenant");
    let adapter_kind = ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter");
    let installation_id =
        ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install");
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            tenant_id.clone(),
            adapter_kind,
            installation_id,
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    let conversation_port: Arc<dyn ironclaw_conversations::ConversationBindingService> =
        conversations;
    let subject_resolver = Arc::new(RecordingSubjectRouteResolver::default());
    let scope = ProductInstallationScope::with_default_scope(
        tenant_id,
        AgentId::new("agent:alpha").expect("agent"),
        Some(ProjectId::new("project:alpha").expect("project")),
    )
    .with_conversation_subject_route_resolver(subject_resolver.clone());
    let resolver = StaticProductInstallationResolver::new([(
        ProductInstallationKey::new(
            ProductAdapterId::new("test_adapter").expect("adapter"),
            AdapterInstallationId::new("install_alpha").expect("installation"),
        ),
        scope,
    )]);
    let binding = ProductConversationBindingService::new(conversation_port.clone(), resolver);
    let envelope = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("installation"),
        ExternalEventId::new("evt:shared-dynamic-route-subject").expect("event"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(Some("T-team"), "C-eng", Some("thread-1"), Some("msg-1"))
            .expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello shared", vec![], ProductTriggerReason::BotMention)
                .expect("message"),
        ),
    );

    let error = binding
        .resolve_binding(ResolveBindingRequest::from_envelope(&envelope))
        .await
        .expect_err("shared binding must require a configured subject");
    assert!(matches!(
        error,
        ProductWorkflowError::BindingRequired { reason }
            if reason == "shared product route requires a configured subject user"
    ));

    subject_resolver.set_subject(UserId::new("user:eng-team").expect("route subject"));
    let resolved = binding
        .resolve_binding(ResolveBindingRequest::from_envelope(&envelope))
        .await
        .expect("shared binding should resolve after host route update");

    assert_eq!(resolved.actor_user_id.as_str(), "user:alice");
    assert_eq!(
        resolved.subject_user_id.as_ref().map(UserId::as_str),
        Some("user:eng-team")
    );

    let failing_subject_resolver = Arc::new(FailingSubjectRouteResolver::default());
    let failing_scope = ProductInstallationScope::with_default_scope(
        TenantId::new("tenant:alpha").expect("tenant"),
        AgentId::new("agent:alpha").expect("agent"),
        Some(ProjectId::new("project:alpha").expect("project")),
    )
    .with_conversation_subject_route_resolver(failing_subject_resolver.clone());
    let failing_installation_resolver = StaticProductInstallationResolver::new([(
        ProductInstallationKey::new(
            ProductAdapterId::new("test_adapter").expect("adapter"),
            AdapterInstallationId::new("install_alpha").expect("installation"),
        ),
        failing_scope,
    )]);
    let failing_binding = ProductConversationBindingService::new(
        conversation_port.clone(),
        failing_installation_resolver,
    );
    let existing_route_envelope = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("installation"),
        ExternalEventId::new("evt:shared-dynamic-route-subject-existing").expect("event"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(
            Some("T-team"),
            "C-eng",
            Some("thread-1"),
            Some("msg-existing"),
        )
        .expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new(
                "hello existing shared thread",
                vec![],
                ProductTriggerReason::BotMention,
            )
            .expect("message"),
        ),
    );
    let resolved_with_unavailable_route_store = failing_binding
        .resolve_binding(ResolveBindingRequest::from_envelope(
            &existing_route_envelope,
        ))
        .await
        .expect("existing shared binding should not need route resolver");

    assert_eq!(
        resolved_with_unavailable_route_store.thread_id,
        resolved.thread_id
    );
    assert_eq!(
        resolved_with_unavailable_route_store
            .subject_user_id
            .as_ref()
            .map(UserId::as_str),
        Some("user:eng-team")
    );
    let route_mismatch_replay = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("installation"),
        ExternalEventId::new("evt:shared-dynamic-route-subject-existing").expect("event"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(
            Some("T-team"),
            "C-ops",
            Some("thread-1"),
            Some("msg-existing"),
        )
        .expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new(
                "reused event id on a different shared route",
                vec![],
                ProductTriggerReason::BotMention,
            )
            .expect("message"),
        ),
    );
    let route_mismatch = failing_binding
        .resolve_binding(ResolveBindingRequest::from_envelope(&route_mismatch_replay))
        .await
        .expect_err("existing shared binding must record the external event route");
    assert!(matches!(
        route_mismatch,
        ProductWorkflowError::BindingAccessDenied
    ));
    assert_eq!(failing_subject_resolver.call_count(), 0);
    let calls = subject_resolver.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].route_key.space_id(), Some("T-team"));
    assert_eq!(calls[0].route_key.conversation_id(), "C-eng");

    subject_resolver.set_subject(UserId::new("user:ops-team").expect("updated route subject"));
    let reassigned_route_envelope = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("installation"),
        ExternalEventId::new("evt:shared-dynamic-route-subject-2").expect("event"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(Some("T-team"), "C-eng", Some("thread-1"), Some("msg-2"))
            .expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new(
                "hello existing shared thread",
                vec![],
                ProductTriggerReason::BotMention,
            )
            .expect("message"),
        ),
    );
    let resolved_after_route_update = binding
        .resolve_binding(ResolveBindingRequest::from_envelope(
            &reassigned_route_envelope,
        ))
        .await
        .expect("existing shared binding should keep its original subject");

    assert_eq!(resolved_after_route_update.thread_id, resolved.thread_id);
    assert_eq!(
        resolved_after_route_update
            .subject_user_id
            .as_ref()
            .map(UserId::as_str),
        Some("user:eng-team")
    );

    subject_resolver.clear_subject();
    let deleted_route_envelope = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("installation"),
        ExternalEventId::new("evt:shared-dynamic-route-subject-3").expect("event"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(Some("T-team"), "C-eng", Some("thread-1"), Some("msg-3"))
            .expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new(
                "hello deleted shared route",
                vec![],
                ProductTriggerReason::BotMention,
            )
            .expect("message"),
        ),
    );
    let resolved_after_route_delete = binding
        .resolve_binding(ResolveBindingRequest::from_envelope(
            &deleted_route_envelope,
        ))
        .await
        .expect("existing shared binding should survive route deletion");

    assert_eq!(resolved_after_route_delete.thread_id, resolved.thread_id);
    assert_eq!(
        resolved_after_route_delete
            .subject_user_id
            .as_ref()
            .map(UserId::as_str),
        Some("user:eng-team")
    );

    let deleted_route_lookup_envelope = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("installation"),
        ExternalEventId::new("evt:shared-dynamic-route-subject-4").expect("event"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(Some("T-team"), "C-eng", Some("thread-1"), Some("msg-4"))
            .expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new(
                "lookup deleted shared route",
                vec![],
                ProductTriggerReason::BotMention,
            )
            .expect("message"),
        ),
    );
    let looked_up_after_route_delete = binding
        .lookup_binding(ResolveBindingRequest::from_envelope(
            &deleted_route_lookup_envelope,
        ))
        .await
        .expect("existing shared binding lookup should survive route deletion");

    assert_eq!(looked_up_after_route_delete.thread_id, resolved.thread_id);
    assert_eq!(
        looked_up_after_route_delete
            .subject_user_id
            .as_ref()
            .map(UserId::as_str),
        Some("user:eng-team")
    );
}

#[tokio::test]
async fn shared_route_can_disable_default_subject_for_unrouted_conversations() {
    let tenant_id = TenantId::new("tenant:alpha").expect("tenant");
    let adapter_kind = ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter");
    let installation_id =
        ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install");
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            tenant_id.clone(),
            adapter_kind,
            installation_id,
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    let conversation_port: Arc<dyn ironclaw_conversations::ConversationBindingService> =
        conversations.clone();
    let actor_pairings: Arc<dyn ironclaw_conversations::ConversationActorPairingService> =
        conversations;
    let subject_resolver = Arc::new(RecordingSubjectRouteResolver::default());
    let actor_resolver = Arc::new(RecordingProductActorUserResolver::new([(
        ExternalActorRef::new("test", "user1", None::<String>).expect("actor"),
        UserId::new("user:alice").expect("user"),
    )]));
    let scope = ProductInstallationScope::with_default_scope(
        tenant_id,
        AgentId::new("agent:alpha").expect("agent"),
        Some(ProjectId::new("project:alpha").expect("project")),
    )
    .with_default_subject_user_id(UserId::new("user:default-team").expect("default subject"))
    .with_conversation_subject_route(
        ProductConversationRouteKey::new(Some("T-team".to_string()), "C-static".to_string())
            .expect("static route key"),
        UserId::new("user:static-team").expect("static route subject"),
    )
    .with_conversation_subject_route_resolver(subject_resolver.clone())
    .without_default_subject_for_unrouted_shared_conversations()
    .with_actor_user_resolver(actor_resolver.clone(), actor_pairings);
    let resolver = StaticProductInstallationResolver::new([(
        ProductInstallationKey::new(
            ProductAdapterId::new("test_adapter").expect("adapter"),
            AdapterInstallationId::new("install_alpha").expect("installation"),
        ),
        scope,
    )]);
    let binding = ProductConversationBindingService::new(conversation_port.clone(), resolver);

    let unrouted = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("installation"),
        ExternalEventId::new("evt:shared-default-disabled").expect("event"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(Some("T-team"), "C-ops", Some("thread-1"), Some("msg-1"))
            .expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello unrouted", vec![], ProductTriggerReason::BotMention)
                .expect("message"),
        ),
    );
    let error = binding
        .resolve_binding(ResolveBindingRequest::from_envelope(&unrouted))
        .await
        .expect_err("unrouted shared binding must not fall back to default subject");
    assert!(matches!(
        error,
        ProductWorkflowError::BindingRequired { reason }
            if reason == "shared product route requires a configured subject user"
    ));
    assert!(
        actor_resolver.calls().is_empty(),
        "unrouted shared route must fail before actor resolver side effects"
    );

    let static_route = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("installation"),
        ExternalEventId::new("evt:shared-static-with-default-disabled").expect("event"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(Some("T-team"), "C-static", Some("thread-1"), Some("msg-2"))
            .expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello static", vec![], ProductTriggerReason::BotMention)
                .expect("message"),
        ),
    );
    let resolved_static = binding
        .resolve_binding(ResolveBindingRequest::from_envelope(&static_route))
        .await
        .expect("static shared route still resolves without default fallback");
    assert_eq!(
        resolved_static.subject_user_id.as_ref().map(UserId::as_str),
        Some("user:static-team")
    );

    subject_resolver.set_subject(UserId::new("user:dynamic-team").expect("dynamic route subject"));
    let dynamic_route = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("installation"),
        ExternalEventId::new("evt:shared-dynamic-with-default-disabled").expect("event"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(Some("T-team"), "C-dyn", Some("thread-1"), Some("msg-3"))
            .expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello dynamic", vec![], ProductTriggerReason::BotMention)
                .expect("message"),
        ),
    );
    let resolved_dynamic = binding
        .resolve_binding(ResolveBindingRequest::from_envelope(&dynamic_route))
        .await
        .expect("dynamic shared route still resolves without default fallback");
    assert_eq!(
        resolved_dynamic
            .subject_user_id
            .as_ref()
            .map(UserId::as_str),
        Some("user:dynamic-team")
    );

    subject_resolver.set_subject(UserId::new("user:reassigned-team").expect("route subject"));
    let reassigned_dynamic_route = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("installation"),
        ExternalEventId::new("evt:shared-dynamic-with-default-disabled-reassigned").expect("event"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(Some("T-team"), "C-dyn", Some("thread-1"), Some("msg-4"))
            .expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new(
                "hello reassigned dynamic",
                vec![],
                ProductTriggerReason::BotMention,
            )
            .expect("message"),
        ),
    );
    let reassigned_error = binding
        .resolve_binding(ResolveBindingRequest::from_envelope(
            &reassigned_dynamic_route,
        ))
        .await
        .expect_err("existing shared binding must not switch subjects without rebinding");
    assert!(matches!(
        reassigned_error,
        ProductWorkflowError::BindingAccessDenied
    ));

    subject_resolver.clear_subject();
    let deleted_dynamic_route = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("installation"),
        ExternalEventId::new("evt:shared-dynamic-with-default-disabled-deleted").expect("event"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(Some("T-team"), "C-dyn", Some("thread-1"), Some("msg-5"))
            .expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new(
                "hello deleted dynamic",
                vec![],
                ProductTriggerReason::BotMention,
            )
            .expect("message"),
        ),
    );
    let error = binding
        .resolve_binding(ResolveBindingRequest::from_envelope(&deleted_dynamic_route))
        .await
        .expect_err("existing shared binding must stop resolving after route removal");
    assert!(matches!(
        error,
        ProductWorkflowError::BindingRequired { reason }
            if reason == "shared product route requires a configured subject user"
    ));

    let deleted_dynamic_route_lookup = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("installation"),
        ExternalEventId::new("evt:shared-dynamic-with-default-disabled-deleted-lookup")
            .expect("event"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(Some("T-team"), "C-dyn", Some("thread-1"), Some("msg-6"))
            .expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new(
                "lookup deleted dynamic",
                vec![],
                ProductTriggerReason::BotMention,
            )
            .expect("message"),
        ),
    );
    let lookup_error = binding
        .lookup_binding(ResolveBindingRequest::from_envelope(
            &deleted_dynamic_route_lookup,
        ))
        .await
        .expect_err("existing shared binding lookup must stop after route removal");
    assert!(matches!(
        lookup_error,
        ProductWorkflowError::BindingRequired { reason }
            if reason == "shared product route requires a configured subject user"
    ));
}

#[tokio::test]
async fn shared_lookup_binding_rejects_existing_binding_when_resolved_actor_differs() {
    let tenant_id = TenantId::new("tenant:alpha").expect("tenant");
    let adapter_kind = ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter");
    let installation_id =
        ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install");
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            tenant_id.clone(),
            adapter_kind.clone(),
            installation_id.clone(),
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    let envelope = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("installation"),
        ExternalEventId::new("evt:seed-shared-lookup").expect("event"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(Some("T-team"), "C-eng", Some("thread-1"), Some("msg-1"))
            .expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello shared", vec![], ProductTriggerReason::BotMention)
                .expect("message"),
        ),
    );
    ConversationBindingPort::resolve_or_create_binding_with_trusted_scope(
        conversations.as_ref(),
        ironclaw_conversations::ResolveConversationRequest {
            tenant_id: tenant_id.clone(),
            adapter_kind,
            adapter_installation_id: installation_id,
            external_actor_ref: ironclaw_conversations::ExternalActorRef::new("test", "user1")
                .expect("actor"),
            external_conversation_ref: ironclaw_conversations::ExternalConversationRef::new(
                Some("T-team"),
                "C-eng",
                Some("thread-1"),
                Some("msg-1"),
            )
            .expect("conversation"),
            external_event_id: ironclaw_conversations::ExternalEventId::new(
                "evt:seed-shared-lookup",
            )
            .expect("event"),
            route_kind: ironclaw_conversations::ConversationRouteKind::Shared,
            requested_agent_id: Some(AgentId::new("agent:alpha").expect("agent")),
            requested_project_id: Some(ProjectId::new("project:alpha").expect("project")),
        },
        Some(AgentId::new("agent:alpha").expect("agent")),
        Some(ProjectId::new("project:alpha").expect("project")),
        Some(UserId::new("user:subject").expect("subject")),
    )
    .await
    .expect("seed binding");
    let conversation_port: Arc<dyn ironclaw_conversations::ConversationBindingService> =
        conversations.clone();
    let actor_pairings: Arc<dyn ironclaw_conversations::ConversationActorPairingService> =
        conversations;
    let scope = ProductInstallationScope::with_default_scope(
        tenant_id,
        AgentId::new("agent:alpha").expect("agent"),
        Some(ProjectId::new("project:alpha").expect("project")),
    )
    .with_preconfigured_actor_binding(
        ExternalActorRef::new("test", "user1", None::<String>).expect("actor"),
        UserId::new("user:bob").expect("user"),
        actor_pairings,
    );
    let resolver = StaticProductInstallationResolver::new([(
        ProductInstallationKey::new(
            ProductAdapterId::new("test_adapter").expect("adapter"),
            AdapterInstallationId::new("install_alpha").expect("installation"),
        ),
        scope,
    )]);
    let binding = ProductConversationBindingService::new(conversation_port, resolver);

    let error = binding
        .lookup_binding(ResolveBindingRequest::from_envelope(&envelope))
        .await
        .expect_err("lookup should reject mismatched resolved actor");

    assert!(matches!(error, ProductWorkflowError::BindingAccessDenied));
}

#[tokio::test]
async fn lookup_binding_does_not_backfill_legacy_ownerless_shared_route() {
    let tenant_id = TenantId::new("tenant:alpha").expect("tenant");
    let adapter_kind = ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter");
    let installation_id =
        ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install");
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            tenant_id.clone(),
            adapter_kind.clone(),
            installation_id.clone(),
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    ConversationBindingPort::resolve_or_create_binding(
        conversations.as_ref(),
        ironclaw_conversations::ResolveConversationRequest {
            tenant_id: tenant_id.clone(),
            adapter_kind,
            adapter_installation_id: installation_id,
            external_actor_ref: ironclaw_conversations::ExternalActorRef::new("test", "user1")
                .expect("actor"),
            external_conversation_ref: ironclaw_conversations::ExternalConversationRef::new(
                Some("T-team"),
                "C-eng",
                Some("thread-legacy"),
                Some("msg-legacy"),
            )
            .expect("conversation"),
            external_event_id: ironclaw_conversations::ExternalEventId::new("evt:legacy-shared")
                .expect("event"),
            route_kind: ironclaw_conversations::ConversationRouteKind::Shared,
            requested_agent_id: Some(AgentId::new("agent:legacy").expect("agent")),
            requested_project_id: Some(ProjectId::new("project:legacy").expect("project")),
        },
    )
    .await
    .expect("seed legacy shared binding");

    let subject_resolver = Arc::new(RecordingSubjectRouteResolver::default());
    subject_resolver.set_subject(UserId::new("user:eng-team").expect("route subject"));
    let scope = ProductInstallationScope::with_default_scope(
        tenant_id,
        AgentId::new("agent:alpha").expect("agent"),
        Some(ProjectId::new("project:alpha").expect("project")),
    )
    .with_conversation_subject_route_resolver(subject_resolver.clone());
    let resolver = StaticProductInstallationResolver::new([(
        ProductInstallationKey::new(
            ProductAdapterId::new("test_adapter").expect("adapter"),
            AdapterInstallationId::new("install_alpha").expect("installation"),
        ),
        scope,
    )]);
    let conversation_port: Arc<dyn ironclaw_conversations::ConversationBindingService> =
        conversations.clone();
    let binding = ProductConversationBindingService::new(conversation_port, resolver);
    let envelope = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("installation"),
        ExternalEventId::new("evt:legacy-shared-lookup").expect("event"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(
            Some("T-team"),
            "C-eng",
            Some("thread-legacy"),
            Some("msg-lookup"),
        )
        .expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new(
                "lookup existing legacy shared route",
                vec![],
                ProductTriggerReason::BotMention,
            )
            .expect("message"),
        ),
    );

    let error = binding
        .lookup_binding(ResolveBindingRequest::from_envelope(&envelope))
        .await
        .expect_err("lookup must not backfill legacy ownerless shared routes");

    assert!(matches!(error, ProductWorkflowError::BindingAccessDenied));
    assert!(
        subject_resolver.calls().is_empty(),
        "existing-only lookup must stay read-only and must not invoke route subject resolution"
    );
}

#[tokio::test]
async fn direct_route_skips_dynamic_subject_route_resolver() {
    let tenant_id = TenantId::new("tenant:alpha").expect("tenant");
    let adapter_kind = ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter");
    let installation_id =
        ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install");
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            tenant_id.clone(),
            adapter_kind,
            installation_id,
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    let conversation_port: Arc<dyn ironclaw_conversations::ConversationBindingService> =
        conversations;
    let subject_resolver = Arc::new(FailingSubjectRouteResolver::default());
    let scope = ProductInstallationScope::with_default_scope(
        tenant_id,
        AgentId::new("agent:alpha").expect("agent"),
        Some(ProjectId::new("project:alpha").expect("project")),
    )
    .with_conversation_subject_route_resolver(subject_resolver.clone());
    let resolver = StaticProductInstallationResolver::new([(
        ProductInstallationKey::new(
            ProductAdapterId::new("test_adapter").expect("adapter"),
            AdapterInstallationId::new("install_alpha").expect("installation"),
        ),
        scope,
    )]);
    let binding = ProductConversationBindingService::new(conversation_port, resolver);
    let envelope = sample_envelope_with_payload(
        "direct-skips-subject-resolver",
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello direct", vec![], ProductTriggerReason::DirectChat)
                .expect("message"),
        ),
    );

    let resolved = binding
        .resolve_binding(ResolveBindingRequest::from_envelope(&envelope))
        .await
        .expect("direct binding should not depend on shared-route resolver");

    assert_eq!(resolved.actor_user_id.as_str(), "user:alice");
    assert_eq!(
        resolved.subject_user_id.as_ref().map(UserId::as_str),
        Some("user:alice")
    );
    assert_eq!(subject_resolver.call_count(), 0);
}

#[tokio::test]
async fn shared_route_propagates_dynamic_subject_route_resolver_error() {
    let tenant_id = TenantId::new("tenant:alpha").expect("tenant");
    let conversation_port: Arc<dyn ironclaw_conversations::ConversationBindingService> =
        Arc::new(InMemoryConversationServices::default());
    let scope = ProductInstallationScope::with_default_scope(
        tenant_id,
        AgentId::new("agent:alpha").expect("agent"),
        Some(ProjectId::new("project:alpha").expect("project")),
    )
    .with_conversation_subject_route_resolver(Arc::new(FailingSubjectRouteResolver::default()));
    let resolver = StaticProductInstallationResolver::new([(
        ProductInstallationKey::new(
            ProductAdapterId::new("test_adapter").expect("adapter"),
            AdapterInstallationId::new("install_alpha").expect("installation"),
        ),
        scope,
    )]);
    let binding = ProductConversationBindingService::new(conversation_port, resolver);
    let envelope = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("installation"),
        ExternalEventId::new("evt:shared-route-resolver-error").expect("event"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(Some("T-team"), "C-eng", Some("thread-1"), Some("msg-1"))
            .expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello shared", vec![], ProductTriggerReason::BotMention)
                .expect("message"),
        ),
    );

    let error = binding
        .resolve_binding(ResolveBindingRequest::from_envelope(&envelope))
        .await
        .expect_err("shared resolver error must propagate");

    assert!(matches!(
        error,
        ProductWorkflowError::Transient { reason }
            if reason == "subject resolver backend down"
    ));
}

#[tokio::test]
async fn concrete_product_workflow_bot_mention_uses_shared_route() {
    let binding = Arc::new(FakeConversationBindingService::new());
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        binding.clone(),
    );

    workflow
        .accept_inbound(sample_envelope_with_payload(
            "shared-owner",
            ProductInboundPayload::UserMessage(
                UserMessagePayload::new("hello shared", vec![], ProductTriggerReason::BotMention)
                    .expect("message"),
            ),
        ))
        .await
        .expect("bot mention accepted");

    let submissions = coordinator.submissions();
    assert_eq!(submissions.len(), 1);
    assert_eq!(
        binding.route_kinds(),
        vec![ironclaw_product_workflow::ProductConversationRouteKind::Shared]
    );
}

#[tokio::test]
async fn concrete_product_workflow_reply_to_bot_requires_existing_binding() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            TenantId::new("tenant:alpha").expect("tenant"),
            ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter"),
            ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install"),
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    let binding = product_binding_service(
        conversations,
        vec![(
            "test_adapter",
            "install_alpha",
            "tenant:alpha",
            "agent:alpha",
            Some("project:alpha"),
        )],
    );
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );

    let err = workflow
        .accept_inbound(sample_envelope_with_context(
            ProductAdapterId::new("test_adapter").expect("adapter"),
            AdapterInstallationId::new("install_alpha").expect("install"),
            ExternalEventId::new("evt:random-thread-reply").expect("event"),
            ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
            ExternalConversationRef::new(
                Some("space1"),
                "conv1",
                Some("thread-never-linked"),
                Some("msg1"),
            )
            .expect("conversation"),
            ProductInboundPayload::UserMessage(
                UserMessagePayload::new(
                    "ambient thread reply",
                    vec![],
                    ProductTriggerReason::ReplyToBot,
                )
                .expect("message"),
            ),
        ))
        .await
        .expect_err("reply-to-bot requires a pre-existing linked thread");

    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::ScopeNotFound,
            ..
        }
    ));
    assert!(
        coordinator.submissions().is_empty(),
        "unlinked Slack thread reply must not submit a turn"
    );
}

#[tokio::test]
async fn concrete_product_workflow_reuses_prepared_binding_for_content_only_policy_rewrite() {
    let binding = Arc::new(FakeConversationBindingService::new());
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let policy = Arc::new(FakeBeforeInboundPolicy::new());
    policy.rewrite_user_message(
        UserMessagePayload::new("rewritten direct", vec![], ProductTriggerReason::DirectChat)
            .expect("message"),
    );
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        binding.clone(),
    )
    .with_before_inbound_policy(policy);

    workflow
        .accept_inbound(sample_envelope_with_payload(
            "policy-rewrite-direct-route",
            ProductInboundPayload::UserMessage(
                UserMessagePayload::new("hello direct", vec![], ProductTriggerReason::DirectChat)
                    .expect("message"),
            ),
        ))
        .await
        .expect("policy-rewritten message accepted");

    let submissions = coordinator.submissions();
    assert_eq!(submissions.len(), 1);
    assert_eq!(
        submissions[0].scope.tenant_id.as_str(),
        "tenant:install_alpha"
    );
    assert_eq!(submissions[0].actor.user_id.as_str(), "user:user1");
    assert_eq!(
        submissions[0].scope.agent_id.as_ref().map(AgentId::as_str),
        Some("agent:fake")
    );
    assert_eq!(binding.resolve_count(), 1);
    assert_eq!(
        binding.route_kinds(),
        vec![ironclaw_product_workflow::ProductConversationRouteKind::Direct]
    );
}

#[tokio::test]
async fn concrete_product_workflow_recomputes_route_after_policy_rewrites_trigger() {
    let binding = Arc::new(FakeConversationBindingService::new());
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let policy = Arc::new(FakeBeforeInboundPolicy::new());
    policy.rewrite_user_message(
        UserMessagePayload::new("rewritten shared", vec![], ProductTriggerReason::BotMention)
            .expect("message"),
    );
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        binding.clone(),
    )
    .with_before_inbound_policy(policy);

    workflow
        .accept_inbound(sample_envelope_with_payload(
            "policy-rewrite-shared-route",
            ProductInboundPayload::UserMessage(
                UserMessagePayload::new("hello direct", vec![], ProductTriggerReason::DirectChat)
                    .expect("message"),
            ),
        ))
        .await
        .expect("policy-rewritten message accepted");

    let submissions = coordinator.submissions();
    assert_eq!(submissions.len(), 1);
    assert_eq!(
        binding.route_kinds(),
        vec![
            ironclaw_product_workflow::ProductConversationRouteKind::Direct,
            ironclaw_product_workflow::ProductConversationRouteKind::Shared,
        ]
    );
}

#[tokio::test]
async fn concrete_product_workflow_rejects_unknown_installation_as_terminal() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            TenantId::new("tenant:alpha").expect("tenant"),
            ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter"),
            ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install"),
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    let binding = product_binding_service(conversations, vec![]);
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );
    let envelope = sample_envelope("unknown-install");

    let err = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("unknown installation rejected");
    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::Unauthorized,
            status_code: 403,
            retryable: false,
            ..
        }
    ));
    let duplicate = workflow
        .accept_inbound(envelope)
        .await
        .expect("terminal rejection replays");
    assert!(matches!(duplicate, ProductInboundAck::Duplicate { .. }));
    assert!(coordinator.submissions().is_empty());
}

#[tokio::test]
async fn concrete_product_workflow_rejects_unpaired_actor_before_turn_submission() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    let binding = product_binding_service(
        conversations,
        vec![(
            "test_adapter",
            "install_alpha",
            "tenant:alpha",
            "agent:alpha",
            None,
        )],
    );
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );

    let envelope = sample_envelope("unpaired");
    let err = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("unpaired actor rejected");
    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::ScopeNotFound,
            status_code: 404,
            retryable: false,
            ..
        }
    ));
    assert!(coordinator.submissions().is_empty());

    let duplicate = workflow
        .accept_inbound(envelope)
        .await
        .expect("terminal rejection replays");
    assert!(matches!(duplicate, ProductInboundAck::Duplicate { .. }));
}

#[tokio::test]
async fn terminal_rejection_for_unpaired_actor_does_not_poison_other_actor_event() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            TenantId::new("tenant:alpha").expect("tenant"),
            ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter"),
            ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install"),
            ironclaw_conversations::ExternalActorRef::new("test", "user2").expect("actor"),
            UserId::new("user:bob").expect("user"),
        )
        .await;
    let binding = product_binding_service(
        conversations,
        vec![(
            "test_adapter",
            "install_alpha",
            "tenant:alpha",
            "agent:alpha",
            None,
        )],
    );
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );

    let unpaired = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("install"),
        ExternalEventId::new("evt:shared-event").expect("event"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(None, "conv1", None, None).expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello", vec![], ProductTriggerReason::DirectChat)
                .expect("message"),
        ),
    );
    let err = workflow
        .accept_inbound(unpaired)
        .await
        .expect_err("unpaired actor rejected");
    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::ScopeNotFound,
            ..
        }
    ));

    let valid_other_actor = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("install"),
        ExternalEventId::new("evt:shared-event").expect("event"),
        ExternalActorRef::new("test", "user2", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(None, "conv1", None, None).expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello", vec![], ProductTriggerReason::DirectChat)
                .expect("message"),
        ),
    );
    let accepted = workflow
        .accept_inbound(valid_other_actor)
        .await
        .expect("different actor with same event should not replay rejection");
    assert!(matches!(accepted, ProductInboundAck::Accepted { .. }));
    assert_eq!(coordinator.submissions().len(), 1);
}

#[tokio::test]
async fn accepted_message_replay_validates_current_actor_before_submit() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            TenantId::new("tenant:alpha").expect("tenant"),
            ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter"),
            ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install"),
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    let binding = product_binding_service(
        conversations,
        vec![(
            "test_adapter",
            "install_alpha",
            "tenant:alpha",
            "agent:alpha",
            None,
        )],
    );
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    coordinator.force_thread_busy_once(TurnRunId::new());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );

    let first = sample_envelope("accepted-replay-actor-check");
    let busy = workflow.accept_inbound(first).await.expect("busy ack");
    assert!(matches!(busy, ProductInboundAck::DeferredBusy { .. }));

    let unpaired_retry = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("install"),
        ExternalEventId::new("evt:accepted-replay-actor-check").expect("event"),
        ExternalActorRef::new("test", "user2", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(None, "conv1", None, None).expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello", vec![], ProductTriggerReason::DirectChat)
                .expect("message"),
        ),
    );
    let err = workflow
        .accept_inbound(unpaired_retry)
        .await
        .expect_err("unpaired retry must not replay accepted message");
    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::ScopeNotFound,
            ..
        }
    ));
    assert!(coordinator.submissions().is_empty());
}

#[tokio::test]
async fn concrete_product_workflow_replays_binding_access_denied_rejection() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            TenantId::new("tenant:alpha").expect("tenant"),
            ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter"),
            ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install"),
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    let binding = product_binding_service(
        conversations.clone(),
        vec![(
            "test_adapter",
            "install_alpha",
            "tenant:alpha",
            "agent:alpha",
            None,
        )],
    );
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );
    workflow
        .accept_inbound(sample_envelope("direct-owner"))
        .await
        .expect("owner accepted");
    let direct_thread = coordinator.submissions()[0].scope.thread_id.clone();
    conversations
        .pair_external_actor(
            TenantId::new("tenant:alpha").expect("tenant"),
            ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter"),
            ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install"),
            ironclaw_conversations::ExternalActorRef::new("test", "user2").expect("actor"),
            UserId::new("user:bob").expect("user"),
        )
        .await;
    conversations
        .add_thread_participant(
            &TenantId::new("tenant:alpha").expect("tenant"),
            &direct_thread,
            UserId::new("user:bob").expect("user"),
        )
        .await
        .expect("participant added");
    let denied = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("install"),
        ExternalEventId::new("evt:direct-participant-denied").expect("event"),
        ExternalActorRef::new("test", "user2", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(None, "conv1", None, None).expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new(
                "direct from participant",
                vec![],
                ProductTriggerReason::DirectChat,
            )
            .expect("message"),
        ),
    );

    let err = workflow
        .accept_inbound(denied.clone())
        .await
        .expect_err("direct participant rejected");
    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::Unauthorized,
            status_code: 403,
            retryable: false,
            ..
        }
    ));
    let duplicate = workflow
        .accept_inbound(denied)
        .await
        .expect("terminal rejection replays");
    assert!(matches!(duplicate, ProductInboundAck::Duplicate { .. }));
    assert_eq!(coordinator.submissions().len(), 1);
}

#[tokio::test]
async fn in_memory_idempotency_ledger_reclaims_expired_in_flight_actions() {
    let ledger = InMemoryIdempotencyLedger::with_in_flight_lease(Duration::seconds(10));
    let received_at = Utc::now();
    let fingerprint = ActionFingerprintKey::new(
        ProductAdapterId::new("test_adapter").expect("valid"),
        AdapterInstallationId::new("install_alpha").expect("valid"),
        fingerprint_actor(),
        SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;")
            .expect("valid source binding key"),
        ExternalEventId::new("evt:lease-memory").expect("valid"),
    );

    assert!(matches!(
        ledger
            .begin_or_replay(fingerprint.clone(), received_at)
            .await
            .expect("first reservation"),
        IdempotencyDecision::New(_)
    ));
    let duplicate = ledger
        .begin_or_replay(fingerprint.clone(), received_at + Duration::seconds(5))
        .await
        .expect_err("fresh in-flight action blocks duplicate dispatch");
    assert!(duplicate.to_string().contains("in flight"));
    assert!(matches!(
        ledger
            .begin_or_replay(fingerprint, received_at + Duration::seconds(11))
            .await
            .expect("expired reservation is reclaimed"),
        IdempotencyDecision::New(_)
    ));
}

#[tokio::test]
async fn in_memory_idempotency_ledger_allows_only_one_concurrent_reservation() {
    let ledger = Arc::new(InMemoryIdempotencyLedger::with_in_flight_lease(
        Duration::seconds(10),
    ));
    let received_at = Utc::now();
    let fingerprint = ActionFingerprintKey::new(
        ProductAdapterId::new("test_adapter").expect("valid"),
        AdapterInstallationId::new("install_alpha").expect("valid"),
        fingerprint_actor(),
        SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;")
            .expect("valid source binding key"),
        ExternalEventId::new("evt:lease-concurrent").expect("valid"),
    );
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let first = {
        let ledger = ledger.clone();
        let fingerprint = fingerprint.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            ledger.begin_or_replay(fingerprint, received_at).await
        })
    };
    let second = {
        let ledger = ledger.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            ledger.begin_or_replay(fingerprint, received_at).await
        })
    };

    barrier.wait().await;
    let results = [
        first.await.expect("first task"),
        second.await.expect("second task"),
    ];

    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Ok(IdempotencyDecision::New(_))))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(ProductWorkflowError::Transient { .. })))
            .count(),
        1
    );
}

#[tokio::test]
async fn in_memory_idempotency_ledger_ignores_stale_releases_after_reclaim() {
    let ledger = InMemoryIdempotencyLedger::with_in_flight_lease(Duration::seconds(10));
    let received_at = Utc::now();
    let fingerprint = ActionFingerprintKey::new(
        ProductAdapterId::new("test_adapter").expect("valid"),
        AdapterInstallationId::new("install_alpha").expect("valid"),
        fingerprint_actor(),
        SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;")
            .expect("valid source binding key"),
        ExternalEventId::new("evt:lease-stale").expect("valid"),
    );

    let first = match ledger
        .begin_or_replay(fingerprint.clone(), received_at)
        .await
        .expect("first reservation")
    {
        IdempotencyDecision::New(action) => action,
        IdempotencyDecision::Replay(_) => panic!("expected first reservation"),
    };
    let second = match ledger
        .begin_or_replay(fingerprint.clone(), received_at + Duration::seconds(11))
        .await
        .expect("expired reservation is reclaimed")
    {
        IdempotencyDecision::New(action) => action,
        IdempotencyDecision::Replay(_) => panic!("expected reclaimed reservation"),
    };

    ledger
        .release(first.clone())
        .await
        .expect("stale release is ignored");
    assert!(
        ledger
            .begin_or_replay(fingerprint.clone(), received_at + Duration::seconds(12))
            .await
            .expect_err("new reservation stays protected after stale release")
            .to_string()
            .contains("in flight")
    );

    let mut stale_settle = first.clone();
    stale_settle.settle(ProductInboundAck::NoOp);
    let stale_settle_err = ledger
        .settle(stale_settle)
        .await
        .expect_err("stale settle fails loudly");
    assert!(stale_settle_err.to_string().contains("superseded"));
    assert!(
        ledger
            .begin_or_replay(fingerprint.clone(), received_at + Duration::seconds(12))
            .await
            .expect_err("new reservation stays protected after stale settle")
            .to_string()
            .contains("in flight")
    );

    let mut current_settle = second;
    current_settle.settle(ProductInboundAck::NoOp);
    ledger
        .settle(current_settle)
        .await
        .expect("current reservation settles");
    let mut stale_after_current_settle = first;
    stale_after_current_settle.settle(ProductInboundAck::NoOp);
    let stale_after_current_err = ledger
        .settle(stale_after_current_settle)
        .await
        .expect_err("stale settle remains rejected after current settle");
    assert!(stale_after_current_err.to_string().contains("superseded"));
    assert!(matches!(
        ledger
            .begin_or_replay(fingerprint, received_at + Duration::seconds(12))
            .await
            .expect("settled action replays"),
        IdempotencyDecision::Replay(_)
    ));
}

#[tokio::test]
async fn in_memory_idempotency_ledger_rejects_settle_after_expiry_without_reclaim() {
    let ledger = InMemoryIdempotencyLedger::with_in_flight_lease(Duration::seconds(10));
    let received_at = Utc::now();
    let fingerprint = ActionFingerprintKey::new(
        ProductAdapterId::new("test_adapter").expect("valid"),
        AdapterInstallationId::new("install_alpha").expect("valid"),
        fingerprint_actor(),
        SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;")
            .expect("valid source binding key"),
        ExternalEventId::new("evt:lease-missing").expect("valid"),
    );

    let mut action = match ledger
        .begin_or_replay(fingerprint, received_at)
        .await
        .expect("first reservation")
    {
        IdempotencyDecision::New(action) => action,
        IdempotencyDecision::Replay(_) => panic!("expected first reservation"),
    };
    assert_eq!(
        ledger
            .expire_in_flight_before(received_at + Duration::seconds(11))
            .expect("expired"),
        1
    );
    action.settle(ProductInboundAck::NoOp);

    let err = ledger
        .settle(action)
        .await
        .expect_err("terminal outcome must not report durable success after expiry");
    assert!(err.to_string().contains("reservation missing"));
}

fn product_binding_service(
    conversations: Arc<InMemoryConversationServices>,
    installations: Vec<(&str, &str, &str, &str, Option<&str>)>,
) -> ProductConversationBindingService {
    let conversation_port: Arc<dyn ironclaw_conversations::ConversationBindingService> =
        conversations;
    let resolver = StaticProductInstallationResolver::new(installations.into_iter().map(
        |(adapter, installation, tenant, agent, project)| {
            (
                ProductInstallationKey::new(
                    ProductAdapterId::new(adapter).expect("adapter"),
                    AdapterInstallationId::new(installation).expect("installation"),
                ),
                ProductInstallationScope::with_default_scope(
                    TenantId::new(tenant).expect("tenant"),
                    AgentId::new(agent).expect("agent"),
                    project.map(|value| ProjectId::new(value).expect("project")),
                )
                .with_default_subject_user_id(
                    UserId::new("user:team-agent").expect("team subject"),
                ),
            )
        },
    ));
    ProductConversationBindingService::new(conversation_port, resolver)
}

fn product_binding_service_with_preconfigured_actor(
    conversations: Arc<InMemoryConversationServices>,
    user_id: &str,
) -> ProductConversationBindingService {
    let conversation_port: Arc<dyn ironclaw_conversations::ConversationBindingService> =
        conversations.clone();
    let actor_pairings: Arc<dyn ironclaw_conversations::ConversationActorPairingService> =
        conversations;
    let scope = ProductInstallationScope::with_default_scope(
        TenantId::new("tenant:alpha").expect("tenant"),
        AgentId::new("agent:alpha").expect("agent"),
        Some(ProjectId::new("project:alpha").expect("project")),
    )
    .with_preconfigured_actor_binding(
        ExternalActorRef::new("test", "user1", None::<String>).expect("actor"),
        UserId::new(user_id).expect("user"),
        actor_pairings,
    );
    let resolver = StaticProductInstallationResolver::new([(
        ProductInstallationKey::new(
            ProductAdapterId::new("test_adapter").expect("adapter"),
            AdapterInstallationId::new("install_alpha").expect("installation"),
        ),
        scope,
    )]);
    ProductConversationBindingService::new(conversation_port, resolver)
}

fn product_binding_service_with_actor_user_resolver(
    conversations: Arc<InMemoryConversationServices>,
    bindings: impl IntoIterator<Item = (ExternalActorRef, UserId)>,
) -> (
    ProductConversationBindingService,
    Arc<RecordingProductActorUserResolver>,
) {
    let actor_resolver = Arc::new(RecordingProductActorUserResolver::new(bindings));
    let binding =
        product_binding_service_with_actor_user_resolver_arc(conversations, actor_resolver.clone());
    (binding, actor_resolver)
}

fn product_binding_service_with_actor_user_resolver_arc(
    conversations: Arc<InMemoryConversationServices>,
    actor_resolver: Arc<dyn ProductActorUserResolver>,
) -> ProductConversationBindingService {
    let conversation_port: Arc<dyn ironclaw_conversations::ConversationBindingService> =
        conversations.clone();
    let actor_pairings: Arc<dyn ironclaw_conversations::ConversationActorPairingService> =
        conversations;
    let scope = ProductInstallationScope::with_default_scope(
        TenantId::new("tenant:alpha").expect("tenant"),
        AgentId::new("agent:alpha").expect("agent"),
        Some(ProjectId::new("project:alpha").expect("project")),
    )
    .with_actor_user_resolver(actor_resolver.clone(), actor_pairings);
    let resolver = StaticProductInstallationResolver::new([(
        ProductInstallationKey::new(
            ProductAdapterId::new("test_adapter").expect("adapter"),
            AdapterInstallationId::new("install_alpha").expect("installation"),
        ),
        scope,
    )]);
    ProductConversationBindingService::new(conversation_port, resolver)
}

#[derive(Debug)]
struct RecordingProductActorUserResolver {
    bindings: HashMap<ExternalActorRef, UserId>,
    calls: Mutex<Vec<ProductActorUserResolutionRequest>>,
}

impl RecordingProductActorUserResolver {
    fn new(bindings: impl IntoIterator<Item = (ExternalActorRef, UserId)>) -> Self {
        Self {
            bindings: bindings.into_iter().collect(),
            calls: Mutex::default(),
        }
    }

    fn calls(&self) -> Vec<ProductActorUserResolutionRequest> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl ProductActorUserResolver for RecordingProductActorUserResolver {
    async fn resolve_product_actor_user(
        &self,
        request: ProductActorUserResolutionRequest,
    ) -> Result<Option<UserId>, ProductWorkflowError> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request.clone());
        Ok(self.bindings.get(&request.external_actor_ref).cloned())
    }
}

#[derive(Debug, Default)]
struct RecordingSubjectRouteResolver {
    subject_user_id: Mutex<Option<UserId>>,
    calls: Mutex<Vec<ProductConversationSubjectRouteResolutionRequest>>,
}

struct CountingConversationBindingService {
    inner: Arc<InMemoryConversationServices>,
    lookup_count: AtomicUsize,
    trusted_resolve_count: AtomicUsize,
}

impl CountingConversationBindingService {
    fn new(inner: Arc<InMemoryConversationServices>) -> Self {
        Self {
            inner,
            lookup_count: AtomicUsize::new(0),
            trusted_resolve_count: AtomicUsize::new(0),
        }
    }

    fn lookup_count(&self) -> usize {
        self.lookup_count.load(Ordering::SeqCst)
    }

    fn trusted_resolve_count(&self) -> usize {
        self.trusted_resolve_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl ironclaw_conversations::ConversationBindingService for CountingConversationBindingService {
    async fn resolve_or_create_binding(
        &self,
        request: ironclaw_conversations::ResolveConversationRequest,
    ) -> Result<
        ironclaw_conversations::ConversationBindingResolution,
        ironclaw_conversations::InboundTurnError,
    > {
        self.inner.resolve_or_create_binding(request).await
    }

    async fn resolve_or_create_binding_with_trusted_scope(
        &self,
        request: ironclaw_conversations::ResolveConversationRequest,
        trusted_agent_id: Option<AgentId>,
        trusted_project_id: Option<ProjectId>,
        trusted_owner_user_id: Option<UserId>,
    ) -> Result<
        ironclaw_conversations::ConversationBindingResolution,
        ironclaw_conversations::InboundTurnError,
    > {
        self.trusted_resolve_count.fetch_add(1, Ordering::SeqCst);
        self.inner
            .resolve_or_create_binding_with_trusted_scope(
                request,
                trusted_agent_id,
                trusted_project_id,
                trusted_owner_user_id,
            )
            .await
    }

    async fn lookup_binding(
        &self,
        request: ironclaw_conversations::ResolveConversationRequest,
    ) -> Result<
        ironclaw_conversations::ConversationBindingResolution,
        ironclaw_conversations::InboundTurnError,
    > {
        self.lookup_count.fetch_add(1, Ordering::SeqCst);
        self.inner.lookup_binding(request).await
    }

    async fn link_conversation_to_thread(
        &self,
        request: ironclaw_conversations::LinkConversationRequest,
    ) -> Result<
        ironclaw_conversations::LinkedConversationBinding,
        ironclaw_conversations::InboundTurnError,
    > {
        self.inner.link_conversation_to_thread(request).await
    }

    async fn validate_reply_target(
        &self,
        request: ironclaw_conversations::ValidateReplyTargetRequest,
    ) -> Result<ironclaw_conversations::ReplyTargetBinding, ironclaw_conversations::InboundTurnError>
    {
        self.inner.validate_reply_target(request).await
    }
}

impl RecordingSubjectRouteResolver {
    fn set_subject(&self, subject_user_id: UserId) {
        *self
            .subject_user_id
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(subject_user_id);
    }

    fn clear_subject(&self) {
        *self
            .subject_user_id
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    fn calls(&self) -> Vec<ProductConversationSubjectRouteResolutionRequest> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[async_trait]
impl ProductConversationSubjectRouteResolver for RecordingSubjectRouteResolver {
    async fn resolve_product_conversation_subject_route(
        &self,
        request: ProductConversationSubjectRouteResolutionRequest,
    ) -> Result<Option<UserId>, ProductWorkflowError> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request);
        Ok(self
            .subject_user_id
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone())
    }
}

#[derive(Debug, Default)]
struct FailingSubjectRouteResolver {
    calls: Mutex<usize>,
}

impl FailingSubjectRouteResolver {
    fn call_count(&self) -> usize {
        *self
            .calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl ProductConversationSubjectRouteResolver for FailingSubjectRouteResolver {
    async fn resolve_product_conversation_subject_route(
        &self,
        _request: ProductConversationSubjectRouteResolutionRequest,
    ) -> Result<Option<UserId>, ProductWorkflowError> {
        *self
            .calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) += 1;
        Err(ProductWorkflowError::Transient {
            reason: "subject resolver backend down".into(),
        })
    }
}

#[derive(Debug)]
struct FailingProductActorUserResolver;

#[async_trait]
impl ProductActorUserResolver for FailingProductActorUserResolver {
    async fn resolve_product_actor_user(
        &self,
        _request: ProductActorUserResolutionRequest,
    ) -> Result<Option<UserId>, ProductWorkflowError> {
        Err(ProductWorkflowError::BindingResolutionFailed {
            reason: "actor resolver backend down".into(),
        })
    }
}

#[tokio::test]
async fn duplicate_envelope_replays_prior_outcome() {
    let (workflow, inbound, _ledger) = build_workflow();

    // First submission.
    let envelope = sample_envelope("dup1");
    let first_ack = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("first accept");
    assert!(matches!(first_ack, ProductInboundAck::Accepted { .. }));
    assert_eq!(inbound.accepted_count(), 1);

    // Second submission of same envelope.
    let second_ack = workflow
        .accept_inbound(envelope)
        .await
        .expect("second accept");
    assert!(matches!(second_ack, ProductInboundAck::Duplicate { .. }));
    // InboundTurnService should NOT be called a second time.
    assert_eq!(inbound.accepted_count(), 1);
}

#[tokio::test]
async fn settled_user_message_records_actual_submitted_run_id() {
    let (workflow, _inbound, ledger) = build_workflow();
    let envelope = sample_envelope("run-id");

    let ack = workflow.accept_inbound(envelope).await.expect("accept");
    let ProductInboundAck::Accepted {
        submitted_run_id, ..
    } = ack
    else {
        panic!("expected accepted ack");
    };
    let actions = ledger.settled_actions();
    assert_eq!(actions.len(), 1);
    assert_eq!(
        actions[0].dispatch_kind,
        Some(ActionDispatchKind::UserMessageTurn {
            run_id: submitted_run_id
        })
    );
}

#[tokio::test]
async fn retryable_dispatch_failure_releases_fingerprint_for_recovery() {
    let (workflow, inbound, ledger) = build_workflow();
    inbound.force_failure(ProductWorkflowError::Transient {
        reason: "turn coordinator unavailable".into(),
    });

    let envelope = sample_envelope("transient-released");
    let first_err = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("first attempt should be retryable");
    assert!(first_err.is_retryable());
    assert_eq!(inbound.attempt_count(), 1);
    assert_eq!(ledger.in_flight_count(), 0);

    let second_err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("released fingerprint should retry dispatch");
    assert!(second_err.is_retryable());
    assert_eq!(inbound.attempt_count(), 2);
    assert_eq!(ledger.settled_count(), 0);
}

#[tokio::test]
async fn deferred_busy_is_not_settled_and_retry_can_submit_same_message() {
    let (workflow, inbound, ledger) = build_workflow();
    let accepted_message_ref = AcceptedMessageRef::new("msg:busy-retry").expect("valid msg ref");
    let busy_run = TurnRunId::new();
    inbound.program_outcome(InboundTurnOutcome::DeferredBusy {
        accepted_message_ref: accepted_message_ref.clone(),
        active_run_id: busy_run,
        binding: fake_binding(),
    });

    let envelope = sample_envelope("busy-retry");
    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("busy ack");
    assert!(matches!(first, ProductInboundAck::DeferredBusy { .. }));
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);

    let submitted_run_id = TurnRunId::new();
    inbound.program_outcome(InboundTurnOutcome::Submitted {
        accepted_message_ref: accepted_message_ref.clone(),
        submitted_run_id,
        binding: fake_binding(),
    });
    let second = workflow
        .accept_inbound(envelope)
        .await
        .expect("retry submit");
    assert!(matches!(
        second,
        ProductInboundAck::Accepted {
            submitted_run_id: run_id,
            ..
        } if run_id == submitted_run_id
    ));
    assert_eq!(inbound.attempt_count(), 2);
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn fake_ledger_expiration_reclaims_in_flight_fingerprint() {
    let ledger = FakeIdempotencyLedger::new();
    let received_at = Utc::now();
    let fingerprint = ActionFingerprintKey::new(
        ProductAdapterId::new("test_adapter").expect("valid"),
        AdapterInstallationId::new("install_alpha").expect("valid"),
        fingerprint_actor(),
        SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;").expect("valid"),
        ExternalEventId::new("evt:lease").expect("valid"),
    );

    let first = ledger
        .begin_or_replay(fingerprint.clone(), received_at)
        .await
        .expect("reserve");
    assert!(matches!(first, IdempotencyDecision::New(_)));
    let duplicate = ledger
        .begin_or_replay(fingerprint.clone(), received_at)
        .await
        .expect_err("fresh in-flight action blocks duplicate dispatch");
    assert!(matches!(duplicate, ProductWorkflowError::Transient { .. }));

    assert_eq!(
        ledger.expire_in_flight_before(received_at + Duration::seconds(1)),
        1
    );
    let reclaimed = ledger
        .begin_or_replay(fingerprint, received_at)
        .await
        .expect("expired fingerprint can be reclaimed");
    assert!(matches!(reclaimed, IdempotencyDecision::New(_)));
}

#[tokio::test]
async fn permanent_turn_submission_failure_settles_terminal_rejection() {
    let (workflow, inbound, ledger) = build_workflow();
    inbound.force_failure(ProductWorkflowError::TurnSubmissionFailed {
        error: TurnError::Unauthorized,
    });

    let envelope = sample_envelope("terminal-turn-error");
    let err = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("unauthorized turn rejection should surface error");
    assert!(!err.is_retryable());
    assert_eq!(ledger.settled_count(), 1);

    let replay = workflow
        .accept_inbound(envelope)
        .await
        .expect("terminal rejection should replay duplicate ack");
    let ProductInboundAck::Duplicate { prior } = replay else {
        panic!("expected duplicate replay")
    };
    let ProductInboundAck::Rejected(rejection) = *prior else {
        panic!("expected rejected prior outcome")
    };
    assert_eq!(
        rejection.disposition(),
        ProductRejectionDisposition::Permanent
    );
}

#[tokio::test]
async fn retryable_turn_submission_failure_releases_for_retry() {
    let (workflow, inbound, ledger) = build_workflow();
    inbound.force_failure(ProductWorkflowError::TurnSubmissionFailed {
        error: TurnError::Unavailable {
            reason: "turn store unavailable".into(),
        },
    });

    let envelope = sample_envelope("retryable-turn-error");
    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("unavailable turn rejection should surface retryable error");
    assert!(first.is_retryable());
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);

    let second = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("released retryable turn rejection should dispatch again");
    assert!(second.is_retryable());
    assert_eq!(inbound.attempt_count(), 2);
}

#[tokio::test]
async fn settle_failure_does_not_return_success_ack() {
    let (workflow, inbound, ledger) = build_workflow();
    ledger.force_settle_failure(ironclaw_product_workflow::ProductWorkflowError::Transient {
        reason: "settle timeout".into(),
    });

    let envelope = sample_envelope("settle-fail");
    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("settle failure should fail request");
    assert!(err.is_retryable());
    assert_eq!(inbound.accepted_count(), 1);
    assert_eq!(ledger.settled_count(), 0);
}

#[tokio::test]
async fn unsupported_action_is_settled_as_terminal_rejection() {
    let (workflow, _inbound, ledger) = build_workflow();
    let envelope = sample_noop_envelope("unsupported-base");
    let context = TrustedInboundContext::from_verified_evidence(
        envelope.adapter_id().clone(),
        envelope.installation_id().clone(),
        Utc::now(),
        &ProtocolAuthEvidence::test_verified(
            AuthRequirement::SharedSecretHeader {
                header_name: "X-Secret".into(),
            },
            "install_alpha",
        ),
    )
    .expect("verified");
    let parsed = ParsedProductInbound::new(
        ExternalEventId::new("evt:unsupported").expect("valid"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("valid"),
        ExternalConversationRef::new(None, "conv1", None, None).expect("valid"),
        ProductInboundPayload::LinkedThreadAction(
            LinkedThreadActionPayload::new("action:unsupported", None, None).expect("valid"),
        ),
    )
    .expect("parsed");
    let unsupported =
        ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope");

    let err = workflow
        .accept_inbound(unsupported.clone())
        .await
        .expect_err("unsupported should error");
    assert!(!err.is_retryable());
    assert_eq!(ledger.settled_count(), 1);

    let replay = workflow
        .accept_inbound(unsupported)
        .await
        .expect("duplicate replay");
    assert!(matches!(replay, ProductInboundAck::Duplicate { .. }));
}

#[tokio::test]
async fn ledger_transient_failure_surfaces_retryable_error() {
    let (workflow, _inbound, ledger) = build_workflow();
    ledger.force_failure(ironclaw_product_workflow::ProductWorkflowError::Transient {
        reason: "db timeout".into(),
    });

    let envelope = sample_envelope("fail1");
    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("should fail");
    assert!(err.is_retryable());
}
