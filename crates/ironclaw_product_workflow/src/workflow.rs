//! Host-side `ProductWorkflow` implementation.
//!
//! This is the top-level product action orchestrator that dispatches inbound
//! envelopes to the appropriate downstream service based on payload kind.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_auth::{AuthFlowId, CredentialAccountId};
use ironclaw_host_api::ThreadId;
use ironclaw_product_adapters::{
    ApprovalDecision, ExternalConversationRef, ProductAdapterError, ProductInboundAck,
    ProductInboundEnvelope, ProductInboundPayload, ProductProjectionReadInput,
    ProductProjectionSubject, ProductProjectionSubscribeInput, ProductRejection,
    ProductRejectionKind, ProductWorkflow, ProductWorkflowRejectionKind, ProjectionReadRequest,
    ProjectionSubscriptionRequest, RedactedString,
};
use ironclaw_turns::{
    AcceptedMessageRef, AdmissionRejectionReason, GateRef, IdempotencyKey, TurnActor, TurnError,
    TurnErrorCategory, TurnRunId, TurnScope,
};
use sha2::{Digest, Sha256};
use tracing::debug;

use crate::action::{ActionDispatchKind, ActionFingerprintKey, SourceBindingKey};
use crate::approval_interaction::{
    ApprovalInteractionDecision, ApprovalInteractionRejectionKind, ApprovalInteractionService,
    ListPendingApprovalsRequest, RejectingApprovalInteractionService,
    ResolveApprovalInteractionRequest, ResolveApprovalInteractionResponse,
};
use crate::auth_interaction::{
    AuthInteractionDecision, AuthInteractionRejectionKind, AuthInteractionService,
    RejectingAuthInteractionService, ResolveAuthInteractionRequest, ResolveAuthInteractionResponse,
};
use crate::binding::{
    ConversationBindingService, ProductConversationRouteKind, ResolveBindingRequest,
    ResolvedBinding,
};
use crate::binding_ref::{
    DEFAULT_BINDING_REF_RAW_MAX_BYTES, binding_ref_segment, bounded_idempotency_key,
};
use crate::command_dispatch::{
    ProductCommandAdmission, ProductCommandAdmissionService, ProductCommandContext,
    ProductCommandService, RejectingProductCommandAdmissionService, RejectingProductCommandService,
};
use crate::commands::ProductCommand;
use crate::error::ProductWorkflowError;
use crate::inbound_turn::{InboundTurnService, InboundUserMessageDispatch};
use crate::ledger::{IdempotencyDecision, IdempotencyLedger};
use crate::policy::{BeforeInboundPolicy, NoopBeforeInboundPolicy};

/// Host-side implementation of [`ProductWorkflow`] that dispatches inbound
/// envelopes through the idempotency ledger and routes to the appropriate
/// downstream service.
pub struct DefaultProductWorkflow {
    inbound_turn_service: Arc<dyn InboundTurnService>,
    idempotency_ledger: Arc<dyn IdempotencyLedger>,
    before_inbound_policy: Arc<dyn BeforeInboundPolicy>,
    binding_service: Arc<dyn ConversationBindingService>,
    command_admission_service: Arc<dyn ProductCommandAdmissionService>,
    command_service: Arc<dyn ProductCommandService>,
    approval_interaction_service: Arc<dyn ApprovalInteractionService>,
    auth_interaction_service: Arc<dyn AuthInteractionService>,
}

impl DefaultProductWorkflow {
    pub fn new(
        inbound_turn_service: Arc<dyn InboundTurnService>,
        idempotency_ledger: Arc<dyn IdempotencyLedger>,
        binding_service: Arc<dyn ConversationBindingService>,
    ) -> Self {
        Self {
            inbound_turn_service,
            idempotency_ledger,
            before_inbound_policy: Arc::new(NoopBeforeInboundPolicy),
            binding_service,
            command_admission_service: Arc::new(RejectingProductCommandAdmissionService),
            command_service: Arc::new(RejectingProductCommandService),
            approval_interaction_service: Arc::new(RejectingApprovalInteractionService),
            auth_interaction_service: Arc::new(RejectingAuthInteractionService),
        }
    }

    pub fn with_before_inbound_policy(
        mut self,
        before_inbound_policy: Arc<dyn BeforeInboundPolicy>,
    ) -> Self {
        self.before_inbound_policy = before_inbound_policy;
        self
    }

    pub fn with_product_command_admission_service(
        mut self,
        command_admission_service: Arc<dyn ProductCommandAdmissionService>,
    ) -> Self {
        self.command_admission_service = command_admission_service;
        self
    }

    pub fn with_product_command_service(
        mut self,
        command_service: Arc<dyn ProductCommandService>,
    ) -> Self {
        self.command_service = command_service;
        self
    }

    pub fn with_approval_interaction_service(
        mut self,
        approval_interaction_service: Arc<dyn ApprovalInteractionService>,
    ) -> Self {
        self.approval_interaction_service = approval_interaction_service;
        self
    }

    pub fn with_auth_interaction_service(
        mut self,
        auth_interaction_service: Arc<dyn AuthInteractionService>,
    ) -> Self {
        self.auth_interaction_service = auth_interaction_service;
        self
    }
}

#[async_trait]
impl ProductWorkflow for DefaultProductWorkflow {
    async fn submit_inbound(
        &self,
        envelope: ProductInboundEnvelope,
    ) -> Result<ProductInboundAck, ProductAdapterError> {
        if matches!(
            envelope.payload(),
            ProductInboundPayload::ProjectionRead(_)
                | ProductInboundPayload::SubscriptionRequest(_)
        ) {
            return Err(ProductAdapterError::WorkflowRejected {
                kind: ProductWorkflowRejectionKind::InvalidRequest,
                status_code: 400,
                retryable: false,
                reason: RedactedString::new(
                    "projection read/subscribe requests must use ProductWorkflow projection doors",
                ),
            });
        }

        let source_binding_key =
            SourceBindingKey::new(envelope.source_binding_key()).map_err(|reason| {
                ProductAdapterError::from(ProductWorkflowError::BindingResolutionFailed { reason })
            })?;
        let fingerprint = ActionFingerprintKey::new(
            envelope.adapter_id().clone(),
            envelope.installation_id().clone(),
            envelope.external_actor_ref().clone(),
            source_binding_key,
            envelope.external_event_id().clone(),
        );

        let decision = self
            .idempotency_ledger
            .begin_or_replay(fingerprint, envelope.received_at())
            .await
            .map_err(ProductAdapterError::from)?;

        match decision {
            IdempotencyDecision::Replay(action) => {
                debug!(
                    action_id = %action.action_id,
                    "replaying prior settled action"
                );
                if let Some(prior_outcome) = action.outcome {
                    return Ok(ProductInboundAck::Duplicate {
                        prior: Box::new(prior_outcome),
                    });
                }
                Err(ProductAdapterError::Internal {
                    detail: ironclaw_product_adapters::RedactedString::new(
                        "settled action missing outcome",
                    ),
                })
            }
            IdempotencyDecision::New(mut action) => {
                let result = dispatch_payload(
                    &envelope,
                    action.action_id,
                    action.fingerprint.clone(),
                    DispatchPorts {
                        inbound_turn_service: &*self.inbound_turn_service,
                        before_inbound_policy: &*self.before_inbound_policy,
                        binding_service: &*self.binding_service,
                        command_admission_service: &*self.command_admission_service,
                        command_service: &*self.command_service,
                        approval_interaction_service: &*self.approval_interaction_service,
                        auth_interaction_service: &*self.auth_interaction_service,
                    },
                )
                .await;

                match result {
                    Ok(dispatched) => {
                        if should_settle_ack(&dispatched.ack) {
                            action.mark_dispatched(dispatched.dispatch_kind);
                            action.settle(dispatched.ack.clone());
                            self.idempotency_ledger.settle(action).await.map_err(|e| {
                                ProductAdapterError::from(ProductWorkflowError::Transient {
                                    reason: format!(
                                        "failed to settle idempotency ledger entry: {e}"
                                    ),
                                })
                            })?;
                        } else {
                            self.idempotency_ledger.release(action).await.map_err(|e| {
                                ProductAdapterError::from(ProductWorkflowError::Transient {
                                    reason: format!(
                                        "failed to release non-terminal idempotency ledger entry: {e}"
                                    ),
                                })
                            })?;
                        }
                        Ok(dispatched.ack)
                    }
                    Err(e) => {
                        if let Some(ack) = terminal_ack_for_error(&e) {
                            action
                                .mark_dispatched(dispatch_kind_from_ack(&ack, envelope.payload())?);
                            action.settle(ack);
                            self.idempotency_ledger.settle(action).await.map_err(|settle_err| {
                                ProductAdapterError::from(ProductWorkflowError::Transient {
                                    reason: format!(
                                        "failed to settle rejected idempotency ledger entry: {settle_err}"
                                    ),
                                })
                            })?;
                        } else {
                            self.idempotency_ledger.release(action).await.map_err(|release_err| {
                                ProductAdapterError::from(ProductWorkflowError::Transient {
                                    reason: format!(
                                        "failed to release retryable idempotency ledger entry: {release_err}"
                                    ),
                                })
                            })?;
                        }
                        Err(ProductAdapterError::from(e))
                    }
                }
            }
        }
    }

    async fn read_projection(
        &self,
        request: ProductProjectionReadInput,
    ) -> Result<ProjectionReadRequest, ProductAdapterError> {
        let ProductProjectionReadInput {
            subject,
            thread_id_hint,
            after_cursor,
            limit,
        } = request;
        let (actor, scope) =
            resolve_projection_subject(&*self.binding_service, &subject, thread_id_hint.as_deref())
                .await?;

        Ok(ProjectionReadRequest {
            actor,
            scope,
            after_cursor,
            limit,
        })
    }

    async fn subscribe_projection(
        &self,
        request: ProductProjectionSubscribeInput,
    ) -> Result<ProjectionSubscriptionRequest, ProductAdapterError> {
        let ProductProjectionSubscribeInput {
            subject,
            thread_id_hint,
            after_cursor,
        } = request;
        let (actor, scope) =
            resolve_projection_subject(&*self.binding_service, &subject, thread_id_hint.as_deref())
                .await?;

        Ok(ProjectionSubscriptionRequest {
            actor,
            scope,
            after_cursor,
        })
    }
}

struct DispatchedAction {
    ack: ProductInboundAck,
    dispatch_kind: ActionDispatchKind,
}

struct DispatchPorts<'a> {
    inbound_turn_service: &'a dyn InboundTurnService,
    before_inbound_policy: &'a dyn BeforeInboundPolicy,
    binding_service: &'a dyn ConversationBindingService,
    command_admission_service: &'a dyn ProductCommandAdmissionService,
    command_service: &'a dyn ProductCommandService,
    approval_interaction_service: &'a dyn ApprovalInteractionService,
    auth_interaction_service: &'a dyn AuthInteractionService,
}

fn resolve_binding_request(envelope: &ProductInboundEnvelope) -> ResolveBindingRequest {
    ResolveBindingRequest::from_envelope(envelope)
}

async fn resolve_projection_subject(
    binding_service: &dyn ConversationBindingService,
    subject: &ProductProjectionSubject,
    thread_id_hint: Option<&str>,
) -> Result<(TurnActor, TurnScope), ProductAdapterError> {
    match subject {
        ProductProjectionSubject::AdapterExternalRefs {
            adapter_id,
            installation_id,
            external_event_id,
            external_actor_ref,
            external_conversation_ref,
            auth_claim,
        } => {
            let binding = binding_service
                .lookup_binding(ResolveBindingRequest {
                    adapter_id: adapter_id.clone(),
                    installation_id: installation_id.clone(),
                    external_actor_ref: external_actor_ref.clone(),
                    external_conversation_ref: external_conversation_ref.clone(),
                    external_event_id: external_event_id.clone(),
                    route_kind: ProductConversationRouteKind::Direct,
                    auth_claim: auth_claim.clone(),
                })
                .await
                .map_err(ProductAdapterError::from)?;
            let thread_id = projection_thread_id_from_binding(&binding, thread_id_hint)?;
            Ok((
                TurnActor::new(binding.actor_user_id.clone()),
                turn_scope_for_thread(&binding, thread_id),
            ))
        }
        ProductProjectionSubject::CanonicalProjection { actor, scope } => {
            validate_projection_thread_hint(&scope.thread_id, thread_id_hint)?;
            Ok((actor.clone(), scope.clone()))
        }
    }
}

async fn lookup_interaction_binding(
    envelope: &ProductInboundEnvelope,
    binding_service: &dyn ConversationBindingService,
) -> Result<ResolvedBinding, ProductWorkflowError> {
    let request = resolve_binding_request(envelope);
    match binding_service.lookup_binding(request.clone()).await {
        Ok(binding) => Ok(binding),
        Err(ProductWorkflowError::BindingRequired { .. })
            if can_fallback_to_direct_base_binding(&request) =>
        {
            binding_service
                .lookup_binding(direct_base_binding_request(request)?)
                .await
        }
        Err(error) => Err(error),
    }
}

fn can_fallback_to_direct_base_binding(request: &ResolveBindingRequest) -> bool {
    request.route_kind == ProductConversationRouteKind::Direct
        && request.external_conversation_ref.topic_id().is_some()
}

fn direct_base_binding_request(
    mut request: ResolveBindingRequest,
) -> Result<ResolveBindingRequest, ProductWorkflowError> {
    request.external_conversation_ref = ExternalConversationRef::new(
        request.external_conversation_ref.space_id(),
        request.external_conversation_ref.conversation_id(),
        None,
        request.external_conversation_ref.reply_target_message_id(),
    )
    .map_err(|error| ProductWorkflowError::InvalidBindingRequest {
        reason: error.to_string(),
    })?;
    Ok(request)
}

fn projection_thread_id_from_binding(
    binding: &ResolvedBinding,
    thread_id_hint: Option<&str>,
) -> Result<ironclaw_host_api::ThreadId, ProductAdapterError> {
    validate_projection_thread_hint(&binding.thread_id, thread_id_hint)?;
    Ok(binding.thread_id.clone())
}

fn validate_projection_thread_hint(
    expected_thread_id: &ThreadId,
    thread_id_hint: Option<&str>,
) -> Result<(), ProductAdapterError> {
    if let Some(thread_id_hint) = thread_id_hint {
        let hinted = ThreadId::new(thread_id_hint).map_err(|_| {
            ProductAdapterError::MalformedInboundPayload {
                reason: RedactedString::new("invalid thread_id_hint"),
            }
        })?;
        if &hinted != expected_thread_id {
            return Err(ProductAdapterError::WorkflowRejected {
                kind: ProductWorkflowRejectionKind::InvalidRequest,
                status_code: 400,
                retryable: false,
                reason: RedactedString::new(
                    "thread_id_hint does not match resolved projection thread",
                ),
            });
        }
    }
    Ok(())
}

async fn dispatch_payload(
    envelope: &ProductInboundEnvelope,
    action_id: crate::ProductActionId,
    action_fingerprint: ActionFingerprintKey,
    ports: DispatchPorts<'_>,
) -> Result<DispatchedAction, ProductWorkflowError> {
    match envelope.payload() {
        ProductInboundPayload::UserMessage(_) => {
            match ports
                .inbound_turn_service
                .accept_user_message_with_before_policy(envelope, ports.before_inbound_policy)
                .await?
            {
                InboundUserMessageDispatch::Accepted(outcome) => {
                    let ack = outcome.to_ack();
                    let dispatch_kind = dispatch_kind_from_ack(&ack, envelope.payload())?;
                    Ok(DispatchedAction { ack, dispatch_kind })
                }
                InboundUserMessageDispatch::Rejected(rejection) => {
                    debug!(
                        rejection_kind = ?rejection.kind,
                        disposition = ?rejection.disposition(),
                        "before-inbound policy rejected user message"
                    );
                    let ack = ProductInboundAck::Rejected(rejection);
                    let dispatch_kind = dispatch_kind_from_ack(&ack, envelope.payload())?;
                    Ok(DispatchedAction { ack, dispatch_kind })
                }
            }
        }
        ProductInboundPayload::Command(cmd) => {
            let context =
                ProductCommandContext::from_envelope(envelope, action_id, action_fingerprint)?;
            let command = match ProductCommand::from_payload(cmd) {
                Ok(command) => command,
                Err(rejection) => {
                    let ack = ProductInboundAck::Rejected(rejection);
                    let dispatch_kind = dispatch_kind_from_ack(&ack, envelope.payload())?;
                    return Ok(DispatchedAction { ack, dispatch_kind });
                }
            };
            match ports
                .command_admission_service
                .admit(&context, &command)
                .await?
            {
                ProductCommandAdmission::Allowed => {}
                ProductCommandAdmission::Rejected(rejection) => {
                    let ack = ProductInboundAck::Rejected(rejection);
                    let dispatch_kind = dispatch_kind_from_ack(&ack, envelope.payload())?;
                    return Ok(DispatchedAction { ack, dispatch_kind });
                }
            }
            let ack = ports.command_service.execute(context, command).await?;
            let dispatch_kind = dispatch_kind_from_command_ack(&ack, envelope.payload())?;
            Ok(DispatchedAction { ack, dispatch_kind })
        }
        ProductInboundPayload::ApprovalResolution(payload) => {
            dispatch_approval_resolution(
                envelope,
                payload,
                action_fingerprint,
                ports.binding_service,
                ports.approval_interaction_service,
            )
            .await
        }
        ProductInboundPayload::ScopedApprovalResolution(payload) => {
            dispatch_scoped_approval_resolution(
                envelope,
                payload,
                action_fingerprint,
                ports.binding_service,
                ports.approval_interaction_service,
            )
            .await
        }
        ProductInboundPayload::AuthResolution(payload) => {
            dispatch_auth_resolution(
                envelope,
                payload,
                action_fingerprint,
                ports.binding_service,
                ports.auth_interaction_service,
            )
            .await
        }
        ProductInboundPayload::ProjectionRead(_) => {
            Err(ProductWorkflowError::UnsupportedActionKind {
                kind: "projection_read".into(),
            })
        }
        ProductInboundPayload::SubscriptionRequest(_) => {
            Err(ProductWorkflowError::UnsupportedActionKind {
                kind: "subscription_request".into(),
            })
        }
        ProductInboundPayload::ControlAction(_) => Ok(DispatchedAction {
            ack: ProductInboundAck::Rejected(ProductRejection::permanent(
                ProductRejectionKind::InvalidRequest,
                "control action is not supported by this ProductWorkflow implementation",
            )),
            dispatch_kind: ActionDispatchKind::Rejected {
                kind: ProductRejectionKind::InvalidRequest,
            },
        }),
        ProductInboundPayload::LinkedThreadAction(_) => {
            Err(ProductWorkflowError::UnsupportedActionKind {
                kind: "linked_thread_action".into(),
            })
        }
        ProductInboundPayload::NoOp => Ok(DispatchedAction {
            ack: ProductInboundAck::NoOp,
            dispatch_kind: ActionDispatchKind::NoOp,
        }),
    }
}

async fn dispatch_approval_resolution(
    envelope: &ProductInboundEnvelope,
    payload: &ironclaw_product_adapters::ApprovalResolutionPayload,
    action_fingerprint: ActionFingerprintKey,
    binding_service: &dyn ConversationBindingService,
    approval_interaction_service: &dyn ApprovalInteractionService,
) -> Result<DispatchedAction, ProductWorkflowError> {
    let decision = approval_interaction_decision(payload.decision)?;
    let binding = lookup_interaction_binding(envelope, binding_service).await?;
    let scope = turn_scope_from_binding(&binding);
    let actor = TurnActor::new(binding.actor_user_id.clone());
    let gate_ref = GateRef::new(payload.gate_ref.clone()).map_err(|_| {
        ProductWorkflowError::ApprovalInteractionRejected {
            kind: ApprovalInteractionRejectionKind::InvalidGateRef,
        }
    })?;
    let idempotency_key = approval_resolution_idempotency_key(&action_fingerprint)?;
    let response = approval_interaction_service
        .resolve(ResolveApprovalInteractionRequest {
            scope,
            actor,
            run_id_hint: None,
            gate_ref,
            decision,
            idempotency_key,
        })
        .await?;
    let submitted_run_id = run_id_from_approval_resolution(response);
    Ok(DispatchedAction {
        ack: ProductInboundAck::Accepted {
            accepted_message_ref: interaction_accepted_message_ref("approval", envelope)?,
            submitted_run_id,
        },
        dispatch_kind: ActionDispatchKind::try_from_payload(envelope.payload())?,
    })
}

async fn dispatch_scoped_approval_resolution(
    envelope: &ProductInboundEnvelope,
    payload: &ironclaw_product_adapters::ScopedApprovalResolutionPayload,
    action_fingerprint: ActionFingerprintKey,
    binding_service: &dyn ConversationBindingService,
    approval_interaction_service: &dyn ApprovalInteractionService,
) -> Result<DispatchedAction, ProductWorkflowError> {
    let decision = approval_interaction_decision(payload.decision)?;
    let binding = lookup_interaction_binding(envelope, binding_service).await?;
    let scope = turn_scope_from_binding(&binding);
    let actor = TurnActor::new(binding.actor_user_id.clone());
    let pending = approval_interaction_service
        .list_pending(ListPendingApprovalsRequest {
            scope: scope.clone(),
            actor: actor.clone(),
        })
        .await?;
    let gate = match pending.approvals.as_slice() {
        [gate] => gate,
        [] => {
            return Err(ProductWorkflowError::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::MissingGate,
            });
        }
        _ => {
            return Err(ProductWorkflowError::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::AmbiguousGate,
            });
        }
    };
    let gate_ref = gate.gate_ref.clone();
    let idempotency_key = approval_resolution_idempotency_key(&action_fingerprint)?;
    let response = approval_interaction_service
        .resolve(ResolveApprovalInteractionRequest {
            scope,
            actor,
            run_id_hint: Some(gate.run_id),
            gate_ref: gate_ref.clone(),
            decision,
            idempotency_key,
        })
        .await?;
    let submitted_run_id = run_id_from_approval_resolution(response);
    Ok(DispatchedAction {
        ack: ProductInboundAck::Accepted {
            accepted_message_ref: interaction_accepted_message_ref("approval", envelope)?,
            submitted_run_id,
        },
        dispatch_kind: ActionDispatchKind::ScopedApprovalResolution,
    })
}

fn approval_interaction_decision(
    decision: ApprovalDecision,
) -> Result<ApprovalInteractionDecision, ProductWorkflowError> {
    match decision {
        ApprovalDecision::ApproveOnce => Ok(ApprovalInteractionDecision::ApproveOnce),
        ApprovalDecision::Deny => Ok(ApprovalInteractionDecision::Deny),
        ApprovalDecision::AlwaysAllow => Ok(ApprovalInteractionDecision::AlwaysAllow),
    }
}

async fn dispatch_auth_resolution(
    envelope: &ProductInboundEnvelope,
    payload: &ironclaw_product_adapters::AuthResolutionPayload,
    action_fingerprint: ActionFingerprintKey,
    binding_service: &dyn ConversationBindingService,
    auth_interaction_service: &dyn AuthInteractionService,
) -> Result<DispatchedAction, ProductWorkflowError> {
    let decision = match &payload.result {
        ironclaw_product_adapters::AuthResolutionResult::CredentialProvided { credential_ref } => {
            AuthInteractionDecision::CredentialProvided {
                credential_ref: parse_credential_account_id(credential_ref)?,
            }
        }
        ironclaw_product_adapters::AuthResolutionResult::CallbackCompleted { callback_ref } => {
            AuthInteractionDecision::CallbackCompleted {
                callback_ref: parse_auth_flow_id(callback_ref)?,
            }
        }
        ironclaw_product_adapters::AuthResolutionResult::Denied => AuthInteractionDecision::Deny,
    };
    let binding = lookup_interaction_binding(envelope, binding_service).await?;
    let scope = turn_scope_from_binding(&binding);
    let actor = TurnActor::new(binding.actor_user_id.clone());
    let gate_ref = GateRef::new(payload.auth_request_ref.clone()).map_err(|_| {
        ProductWorkflowError::AuthInteractionRejected {
            kind: AuthInteractionRejectionKind::InvalidGateRef,
        }
    })?;
    let idempotency_key = auth_resolution_idempotency_key(&action_fingerprint)?;
    let response = auth_interaction_service
        .resolve(ResolveAuthInteractionRequest {
            scope,
            actor,
            run_id_hint: None,
            gate_ref,
            decision,
            idempotency_key,
        })
        .await?;
    let submitted_run_id = run_id_from_auth_resolution(response);
    Ok(DispatchedAction {
        ack: ProductInboundAck::Accepted {
            accepted_message_ref: interaction_accepted_message_ref("auth", envelope)?,
            submitted_run_id,
        },
        dispatch_kind: ActionDispatchKind::try_from_payload(envelope.payload())?,
    })
}

fn run_id_from_approval_resolution(response: ResolveApprovalInteractionResponse) -> TurnRunId {
    match response {
        ResolveApprovalInteractionResponse::Approved(response) => response.run_id,
        ResolveApprovalInteractionResponse::Denied(response) => response.run_id,
    }
}

fn run_id_from_auth_resolution(response: ResolveAuthInteractionResponse) -> TurnRunId {
    match response {
        ResolveAuthInteractionResponse::Resumed(response) => response.run_id,
        ResolveAuthInteractionResponse::Canceled(response) => response.run_id,
    }
}

fn interaction_accepted_message_ref(
    kind: &str,
    envelope: &ProductInboundEnvelope,
) -> Result<AcceptedMessageRef, ProductWorkflowError> {
    let mut digest_input = Vec::new();
    digest_input
        .extend_from_slice(b"ironclaw_product_workflow:interaction_accepted_message_ref:v1");
    push_length_prefixed_component(&mut digest_input, kind);
    push_length_prefixed_component(&mut digest_input, envelope.installation_id().as_str());
    push_length_prefixed_component(&mut digest_input, envelope.external_actor_ref().kind());
    push_length_prefixed_component(&mut digest_input, envelope.external_actor_ref().id());
    push_length_prefixed_component(
        &mut digest_input,
        &envelope
            .external_conversation_ref()
            .conversation_fingerprint(),
    );
    push_length_prefixed_component(&mut digest_input, envelope.external_event_id().as_str());

    let stable_ref = lower_hex(&Sha256::digest(&digest_input));
    AcceptedMessageRef::new(format!("interaction:{kind}:{stable_ref}")).map_err(|reason| {
        ProductWorkflowError::TurnSubmissionRejected {
            reason: format!("invalid interaction accepted message ref: {reason}"),
        }
    })
}

fn push_length_prefixed_component(bytes: &mut Vec<u8>, component: &str) {
    let component_bytes = component.as_bytes();
    bytes.extend_from_slice(&(component_bytes.len() as u64).to_be_bytes());
    bytes.extend_from_slice(component_bytes);
}

fn lower_hex(bytes: &[u8]) -> String {
    const LOWER_HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(LOWER_HEX[(byte >> 4) as usize]));
        encoded.push(char::from(LOWER_HEX[(byte & 0x0f) as usize]));
    }
    encoded
}

fn approval_resolution_idempotency_key(
    fingerprint: &ActionFingerprintKey,
) -> Result<IdempotencyKey, ProductWorkflowError> {
    interaction_resolution_idempotency_key("product-approval", fingerprint, || {
        ProductWorkflowError::ApprovalInteractionRejected {
            kind: ApprovalInteractionRejectionKind::InvalidBindingRef,
        }
    })
}

fn auth_resolution_idempotency_key(
    fingerprint: &ActionFingerprintKey,
) -> Result<IdempotencyKey, ProductWorkflowError> {
    interaction_resolution_idempotency_key("product-auth", fingerprint, || {
        ProductWorkflowError::AuthInteractionRejected {
            kind: AuthInteractionRejectionKind::InvalidBindingRef,
        }
    })
}

fn parse_credential_account_id(value: &str) -> Result<CredentialAccountId, ProductWorkflowError> {
    uuid::Uuid::parse_str(value)
        .map(CredentialAccountId::from_uuid)
        .map_err(|_| ProductWorkflowError::AuthInteractionRejected {
            kind: AuthInteractionRejectionKind::InvalidCredentialRef,
        })
}

fn parse_auth_flow_id(value: &str) -> Result<AuthFlowId, ProductWorkflowError> {
    uuid::Uuid::parse_str(value)
        .map(AuthFlowId::from_uuid)
        .map_err(|_| ProductWorkflowError::AuthInteractionRejected {
            kind: AuthInteractionRejectionKind::InvalidCallbackRef,
        })
}

fn interaction_resolution_idempotency_key(
    prefix: &str,
    fingerprint: &ActionFingerprintKey,
    invalid_binding_error: impl FnOnce() -> ProductWorkflowError,
) -> Result<IdempotencyKey, ProductWorkflowError> {
    let raw = format!(
        "{}{}{}{}{}{}",
        binding_ref_segment("adapter", fingerprint.adapter_id.as_str()),
        binding_ref_segment("installation", fingerprint.installation_id.as_str()),
        binding_ref_segment("actor_kind", fingerprint.external_actor_ref.kind()),
        binding_ref_segment("actor_id", fingerprint.external_actor_ref.id()),
        binding_ref_segment("source", fingerprint.source_binding_key.as_str()),
        binding_ref_segment("event", fingerprint.external_event_id.as_str())
    );
    bounded_idempotency_key(prefix, &raw, DEFAULT_BINDING_REF_RAW_MAX_BYTES)
        .map_err(|_| invalid_binding_error())
}

fn turn_scope_from_binding(binding: &ResolvedBinding) -> TurnScope {
    turn_scope_for_thread(binding, binding.thread_id.clone())
}

fn turn_scope_for_thread(binding: &ResolvedBinding, thread_id: ThreadId) -> TurnScope {
    TurnScope::new_with_owner(
        binding.tenant_id.clone(),
        binding.agent_id.clone(),
        binding.project_id.clone(),
        thread_id,
        binding.subject_user_id.clone(),
    )
}

fn dispatch_kind_from_ack(
    ack: &ProductInboundAck,
    payload: &ProductInboundPayload,
) -> Result<ActionDispatchKind, ProductWorkflowError> {
    match ack {
        ProductInboundAck::Accepted {
            submitted_run_id, ..
        } => Ok(ActionDispatchKind::UserMessageTurn {
            run_id: *submitted_run_id,
        }),
        ProductInboundAck::DeferredBusy { active_run_id, .. } => {
            Ok(ActionDispatchKind::UserMessageTurn {
                run_id: *active_run_id,
            })
        }
        ProductInboundAck::Rejected(rejection) => Ok(ActionDispatchKind::Rejected {
            kind: rejection.kind.clone(),
        }),
        _ => ActionDispatchKind::try_from_payload(payload),
    }
}

fn dispatch_kind_from_command_ack(
    ack: &ProductInboundAck,
    payload: &ProductInboundPayload,
) -> Result<ActionDispatchKind, ProductWorkflowError> {
    match ack {
        ProductInboundAck::Accepted { .. } | ProductInboundAck::DeferredBusy { .. } => {
            Err(ProductWorkflowError::UnsupportedActionKind {
                kind: "turn_ack_from_product_command".into(),
            })
        }
        ProductInboundAck::Rejected(rejection) => Ok(ActionDispatchKind::Rejected {
            kind: rejection.kind.clone(),
        }),
        _ => ActionDispatchKind::try_from_payload(payload),
    }
}

fn should_settle_ack(ack: &ProductInboundAck) -> bool {
    !matches!(ack, ProductInboundAck::DeferredBusy { .. }) && ack.is_durable_outcome()
}

fn turn_error_is_retryable(error: &TurnError) -> bool {
    !matches!(error.category(), TurnErrorCategory::CapacityExceeded)
        && matches!(error.adapter_status_code(), 429 | 503)
}

fn rejection_kind_for_turn_error(error: &TurnError) -> ProductRejectionKind {
    match error.category() {
        TurnErrorCategory::Unauthorized => ProductRejectionKind::AccessDenied,
        TurnErrorCategory::ScopeNotFound => ProductRejectionKind::BindingRequired,
        TurnErrorCategory::AdmissionRejected => match error {
            TurnError::AdmissionRejected(rejection)
                if matches!(
                    rejection.reason,
                    AdmissionRejectionReason::Policy | AdmissionRejectionReason::Unauthorized
                ) =>
            {
                ProductRejectionKind::AccessDenied
            }
            _ => ProductRejectionKind::PolicyDenied,
        },
        TurnErrorCategory::ThreadBusy
        | TurnErrorCategory::InvalidRequest
        | TurnErrorCategory::CapacityExceeded
        | TurnErrorCategory::Unavailable
        | TurnErrorCategory::Conflict => ProductRejectionKind::PolicyDenied,
    }
}

fn terminal_ack_for_error(error: &ProductWorkflowError) -> Option<ProductInboundAck> {
    match error {
        ProductWorkflowError::UnknownInstallation => {
            Some(ProductInboundAck::Rejected(ProductRejection::permanent(
                ProductRejectionKind::UnknownInstallation,
                "unknown adapter installation",
            )))
        }
        ProductWorkflowError::BindingRequired { reason } => Some(ProductInboundAck::Rejected(
            ProductRejection::permanent(ProductRejectionKind::BindingRequired, reason.clone()),
        )),
        ProductWorkflowError::BindingAccessDenied => {
            Some(ProductInboundAck::Rejected(ProductRejection::permanent(
                ProductRejectionKind::AccessDenied,
                "binding access denied",
            )))
        }
        ProductWorkflowError::InvalidBindingRequest { reason } => {
            Some(ProductInboundAck::Rejected(ProductRejection::permanent(
                ProductRejectionKind::PolicyDenied,
                reason.clone(),
            )))
        }
        ProductWorkflowError::UnsupportedActionKind { kind } => {
            Some(ProductInboundAck::Rejected(ProductRejection::permanent(
                ProductRejectionKind::PolicyDenied,
                format!("unsupported action kind: {kind}"),
            )))
        }
        ProductWorkflowError::ApprovalInteractionRejected { kind } if !kind.retryable() => {
            Some(ProductInboundAck::Rejected(ProductRejection::permanent(
                rejection_kind_for_approval_interaction(*kind),
                kind.sanitized_reason(),
            )))
        }
        ProductWorkflowError::AuthInteractionRejected { kind } if !kind.retryable() => {
            Some(ProductInboundAck::Rejected(ProductRejection::permanent(
                rejection_kind_for_auth_interaction(*kind),
                kind.sanitized_reason(),
            )))
        }
        ProductWorkflowError::TurnSubmissionFailed { error } if !turn_error_is_retryable(error) => {
            Some(ProductInboundAck::Rejected(ProductRejection::permanent(
                rejection_kind_for_turn_error(error),
                format!("turn submission rejected: {error}"),
            )))
        }
        ProductWorkflowError::BeforeInboundPolicyFailed {
            reason,
            permanent: true,
        } => Some(ProductInboundAck::Rejected(ProductRejection::permanent(
            ProductRejectionKind::PolicyDenied,
            reason.clone(),
        ))),
        ProductWorkflowError::BindingResolutionFailed { .. }
        | ProductWorkflowError::TurnSubmissionRejected { .. }
        | ProductWorkflowError::TurnSubmissionFailed { .. }
        | ProductWorkflowError::TurnResumeRejected { .. }
        | ProductWorkflowError::AuthContinuationRejected { .. }
        | ProductWorkflowError::ApprovalInteractionRejected { .. }
        | ProductWorkflowError::AuthInteractionRejected { .. }
        | ProductWorkflowError::TurnResumeDenied { .. }
        | ProductWorkflowError::Transient { .. }
        | ProductWorkflowError::BeforeInboundPolicyFailed {
            permanent: false, ..
        }
        | ProductWorkflowError::DuplicateAction { .. } => None,
    }
}

fn rejection_kind_for_auth_interaction(kind: AuthInteractionRejectionKind) -> ProductRejectionKind {
    match kind {
        AuthInteractionRejectionKind::MissingAuth => ProductRejectionKind::BindingRequired,
        AuthInteractionRejectionKind::CrossScopeDenied => ProductRejectionKind::AccessDenied,
        AuthInteractionRejectionKind::StaleAuth
        | AuthInteractionRejectionKind::InvalidGateRef
        | AuthInteractionRejectionKind::InvalidCredentialRef
        | AuthInteractionRejectionKind::InvalidCallbackRef
        | AuthInteractionRejectionKind::UnsupportedResult
        | AuthInteractionRejectionKind::FlowUnavailable
        | AuthInteractionRejectionKind::InvalidBindingRef => ProductRejectionKind::PolicyDenied,
    }
}

fn rejection_kind_for_approval_interaction(
    kind: ApprovalInteractionRejectionKind,
) -> ProductRejectionKind {
    match kind {
        ApprovalInteractionRejectionKind::MissingGate => ProductRejectionKind::BindingRequired,
        ApprovalInteractionRejectionKind::CrossScopeDenied => ProductRejectionKind::AccessDenied,
        ApprovalInteractionRejectionKind::AmbiguousGate
        | ApprovalInteractionRejectionKind::StaleGate
        | ApprovalInteractionRejectionKind::InvalidGateRef
        | ApprovalInteractionRejectionKind::AlwaysAllowUnsupported
        | ApprovalInteractionRejectionKind::UnsupportedAction
        | ApprovalInteractionRejectionKind::LeaseTermsUnavailable
        | ApprovalInteractionRejectionKind::ResolverUnavailable
        | ApprovalInteractionRejectionKind::InvalidBindingRef => ProductRejectionKind::PolicyDenied,
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use ironclaw_product_adapters::{
        AdapterInstallationId, AuthRequirement, ExternalActorRef, ExternalConversationRef,
        ExternalEventId, ParsedProductInbound, ProductAdapterId, ProductInboundAck,
        ProductInboundEnvelope, ProductInboundPayload, ProtocolAuthEvidence, TrustedInboundContext,
    };
    use ironclaw_turns::{AcceptedMessageRef, AdmissionRejection, TurnRunId};

    use super::*;

    fn interaction_ref_envelope(
        external_event_id: &str,
        actor_id: &str,
        conversation_id: &str,
    ) -> ProductInboundEnvelope {
        let adapter_id = ProductAdapterId::new("test_adapter").expect("adapter");
        let installation_id = AdapterInstallationId::new("install_alpha").expect("install");
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
        .expect("trusted context");
        let parsed = ParsedProductInbound::new(
            ExternalEventId::new(external_event_id).expect("event"),
            ExternalActorRef::new("test", actor_id, None::<String>).expect("actor"),
            ExternalConversationRef::new(None, conversation_id, None, None).expect("conversation"),
            ProductInboundPayload::NoOp,
        )
        .expect("parsed inbound");
        ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope")
    }

    #[test]
    fn interaction_accepted_message_ref_includes_actor_and_conversation_identity() {
        let base = interaction_ref_envelope("evt:same", "user1", "conv1");
        let other_actor = interaction_ref_envelope("evt:same", "user2", "conv1");
        let other_conversation = interaction_ref_envelope("evt:same", "user1", "conv2");

        let base_ref = interaction_accepted_message_ref("approval", &base).expect("base ref");
        assert_ne!(
            base_ref,
            interaction_accepted_message_ref("approval", &other_actor).expect("actor ref")
        );
        assert_ne!(
            base_ref,
            interaction_accepted_message_ref("approval", &other_conversation)
                .expect("conversation ref")
        );
    }

    #[test]
    fn dispatch_kind_from_ack_uses_submitted_or_active_run_ids() {
        let submitted_run_id = TurnRunId::new();
        let accepted = ProductInboundAck::Accepted {
            accepted_message_ref: AcceptedMessageRef::new("msg:accepted").expect("valid ref"),
            submitted_run_id,
        };
        assert_eq!(
            dispatch_kind_from_ack(&accepted, &ProductInboundPayload::NoOp).expect("kind"),
            ActionDispatchKind::UserMessageTurn {
                run_id: submitted_run_id
            }
        );

        let active_run_id = TurnRunId::new();
        let deferred = ProductInboundAck::DeferredBusy {
            accepted_message_ref: AcceptedMessageRef::new("msg:deferred").expect("valid ref"),
            active_run_id,
        };
        assert_eq!(
            dispatch_kind_from_ack(&deferred, &ProductInboundPayload::NoOp).expect("kind"),
            ActionDispatchKind::UserMessageTurn {
                run_id: active_run_id
            }
        );
    }

    #[test]
    fn terminal_ack_for_error_settles_unsupported_actions() {
        let unsupported = terminal_ack_for_error(&ProductWorkflowError::UnsupportedActionKind {
            kind: "auth_resolution".to_string(),
        })
        .expect("unsupported action is terminal");
        assert!(matches!(
            unsupported,
            ProductInboundAck::Rejected(rejection)
                if rejection.kind == ProductRejectionKind::PolicyDenied
                    && rejection.disposition()
                        == ironclaw_product_adapters::ProductRejectionDisposition::Permanent
        ));
    }

    #[test]
    fn terminal_ack_for_error_maps_non_retryable_turn_categories() {
        let unauthorized = terminal_ack_for_error(&ProductWorkflowError::TurnSubmissionFailed {
            error: TurnError::Unauthorized,
        })
        .expect("unauthorized turn failure is terminal");
        assert!(matches!(
            unauthorized,
            ProductInboundAck::Rejected(rejection)
                if rejection.kind == ProductRejectionKind::AccessDenied
        ));

        let missing_scope = terminal_ack_for_error(&ProductWorkflowError::TurnSubmissionFailed {
            error: TurnError::ScopeNotFound,
        })
        .expect("scope-not-found turn failure is terminal");
        assert!(matches!(
            missing_scope,
            ProductInboundAck::Rejected(rejection)
                if rejection.kind == ProductRejectionKind::BindingRequired
        ));

        let admission_policy =
            terminal_ack_for_error(&ProductWorkflowError::TurnSubmissionFailed {
                error: TurnError::AdmissionRejected(AdmissionRejection::new(
                    AdmissionRejectionReason::Policy,
                )),
            })
            .expect("policy admission failure is terminal");
        assert!(matches!(
            admission_policy,
            ProductInboundAck::Rejected(rejection)
                if rejection.kind == ProductRejectionKind::AccessDenied
        ));

        let capacity_exceeded =
            terminal_ack_for_error(&ProductWorkflowError::TurnSubmissionFailed {
                error: TurnError::capacity_exceeded(
                    ironclaw_turns::TurnCapacityResource::SpawnTreeDescendants,
                    4,
                ),
            })
            .expect("capacity failures are terminal policy outcomes");
        assert!(matches!(
            capacity_exceeded,
            ProductInboundAck::Rejected(rejection)
                if rejection.kind == ProductRejectionKind::PolicyDenied
        ));
    }

    #[test]
    fn terminal_ack_for_error_keeps_retryable_paths_unsettled() {
        assert!(
            terminal_ack_for_error(&ProductWorkflowError::BindingResolutionFailed {
                reason: "binding backend unavailable".to_string(),
            })
            .is_none()
        );
        assert!(
            terminal_ack_for_error(&ProductWorkflowError::Transient {
                reason: "ledger timeout".to_string(),
            })
            .is_none()
        );
        assert!(
            terminal_ack_for_error(&ProductWorkflowError::TurnSubmissionFailed {
                error: TurnError::Unavailable {
                    reason: "turn store unavailable".to_string(),
                },
            })
            .is_none()
        );
    }

    #[test]
    fn terminal_success_ack_excludes_deferred_busy() {
        assert!(should_settle_ack(&ProductInboundAck::NoOp));
        assert!(!should_settle_ack(&ProductInboundAck::DeferredBusy {
            accepted_message_ref: AcceptedMessageRef::new("msg:busy").expect("valid ref"),
            active_run_id: TurnRunId::new(),
        }));
    }

    #[test]
    fn should_settle_ack_respects_rejection_disposition() {
        assert!(should_settle_ack(&ProductInboundAck::Rejected(
            ProductRejection::permanent(ProductRejectionKind::PolicyDenied, "blocked")
        )));
        assert!(!should_settle_ack(&ProductInboundAck::Rejected(
            ProductRejection::retryable(ProductRejectionKind::PolicyDenied, "try later")
        )));
    }
}
