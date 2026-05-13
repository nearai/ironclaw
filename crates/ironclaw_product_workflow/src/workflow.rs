//! Host-side `ProductWorkflow` implementation.
//!
//! This is the top-level product action orchestrator that dispatches inbound
//! envelopes to the appropriate downstream service based on payload kind.
//!
//! Optional service ports (`BeforeInboundPolicy` is held by
//! `DefaultInboundTurnService`, the rest by this struct) are wired through
//! [`DefaultProductWorkflow::new`] plus builder-style `with_*` methods. When a
//! port is unset, the corresponding dispatch arm returns a redacted permanent
//! rejection — adapters must wire services before serving the matching action
//! kind in production.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_product_adapters::{
    LoopGateRef as AdapterLoopGateRef, ProductAdapterError,
    ProductCommandName as AdapterCommandName, ProductInboundAck, ProductInboundEnvelope,
    ProductInboundPayload, ProductRejection, ProductRejectionKind, ProductWorkflow,
    ProjectionSubscriptionRequest, RedactedString,
};
use ironclaw_turns::{AdmissionRejectionReason, TurnError, TurnErrorCategory};
use tracing::debug;

use crate::action::{ActionDispatchKind, ActionFingerprintKey, SourceBindingKey};
use crate::error::ProductWorkflowError;
use crate::inbound_turn::InboundTurnService;
use crate::ledger::{IdempotencyDecision, IdempotencyLedger};
use crate::services::{
    ApprovalInteractionService, ApprovalResolutionOutcome, AuthInteractionService,
    AuthResolutionOutcome, LinkedThreadActionOutcome, LinkedThreadActionService,
    MissionFireOutcome, MissionFireRequest,
    MissionFireSuppressionReason as WorkflowMissionFireSuppressionReason, MissionService,
    ProductCommandOutcome, ProductCommandRouter, ProjectionSubscriptionAuthority,
    ProjectionSubscriptionAuthorityRequest, SystemActionService,
};

/// Host-side implementation of [`ProductWorkflow`] that dispatches inbound
/// envelopes through the idempotency ledger and routes to the appropriate
/// downstream service.
pub struct DefaultProductWorkflow {
    inbound_turn_service: Arc<dyn InboundTurnService>,
    idempotency_ledger: Arc<dyn IdempotencyLedger>,
    command_router: Option<Arc<dyn ProductCommandRouter>>,
    approval_service: Option<Arc<dyn ApprovalInteractionService>>,
    auth_service: Option<Arc<dyn AuthInteractionService>>,
    linked_thread_service: Option<Arc<dyn LinkedThreadActionService>>,
    mission_service: Option<Arc<dyn MissionService>>,
    system_action_service: Option<Arc<dyn SystemActionService>>,
    projection_authority: Option<Arc<dyn ProjectionSubscriptionAuthority>>,
}

impl DefaultProductWorkflow {
    pub fn new(
        inbound_turn_service: Arc<dyn InboundTurnService>,
        idempotency_ledger: Arc<dyn IdempotencyLedger>,
    ) -> Self {
        Self {
            inbound_turn_service,
            idempotency_ledger,
            command_router: None,
            approval_service: None,
            auth_service: None,
            linked_thread_service: None,
            mission_service: None,
            system_action_service: None,
            projection_authority: None,
        }
    }

    pub fn with_command_router(mut self, router: Arc<dyn ProductCommandRouter>) -> Self {
        self.command_router = Some(router);
        self
    }

    pub fn with_approval_service(mut self, service: Arc<dyn ApprovalInteractionService>) -> Self {
        self.approval_service = Some(service);
        self
    }

    pub fn with_auth_service(mut self, service: Arc<dyn AuthInteractionService>) -> Self {
        self.auth_service = Some(service);
        self
    }

    pub fn with_linked_thread_service(
        mut self,
        service: Arc<dyn LinkedThreadActionService>,
    ) -> Self {
        self.linked_thread_service = Some(service);
        self
    }

    pub fn with_mission_service(mut self, service: Arc<dyn MissionService>) -> Self {
        self.mission_service = Some(service);
        self
    }

    pub fn with_system_action_service(mut self, service: Arc<dyn SystemActionService>) -> Self {
        self.system_action_service = Some(service);
        self
    }

    pub fn with_projection_authority(
        mut self,
        authority: Arc<dyn ProjectionSubscriptionAuthority>,
    ) -> Self {
        self.projection_authority = Some(authority);
        self
    }
}

#[async_trait]
impl ProductWorkflow for DefaultProductWorkflow {
    async fn accept_inbound(
        &self,
        envelope: ProductInboundEnvelope,
    ) -> Result<ProductInboundAck, ProductAdapterError> {
        // Subscription requests are read-only — they MUST NOT take a
        // ProductInboundAction ledger row (issue #3280 AC #14). Adapters
        // are expected to call resolve_projection_subscription instead;
        // routing one through accept_inbound is a usage error.
        if matches!(
            envelope.payload(),
            ProductInboundPayload::SubscriptionRequest(_)
        ) {
            return Err(ProductAdapterError::from(
                ProductWorkflowError::UnsupportedActionKind {
                    kind: "subscription_request_via_accept_inbound".into(),
                },
            ));
        }

        let source_binding_key =
            SourceBindingKey::new(envelope.source_binding_key()).map_err(|reason| {
                ProductAdapterError::from(ProductWorkflowError::BindingResolutionFailed { reason })
            })?;
        let fingerprint = ActionFingerprintKey::new(
            envelope.adapter_id().clone(),
            envelope.installation_id().clone(),
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
                    detail: RedactedString::new("settled action missing outcome"),
                })
            }
            IdempotencyDecision::New(mut action) => {
                let result = self.dispatch_payload(&envelope).await;

                match result {
                    Ok(ack) => {
                        action.mark_dispatched(dispatch_kind_from_ack(&ack, envelope.payload())?);
                        if is_terminal_success_ack(&ack) {
                            action.settle(ack.clone());
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
                        Ok(ack)
                    }
                    Err(e) => {
                        if let Some(ack) = terminal_ack_for_error(&e) {
                            action.mark_dispatched(ActionDispatchKind::try_from_payload(
                                envelope.payload(),
                            )?);
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

    async fn resolve_projection_subscription(
        &self,
        envelope: ProductInboundEnvelope,
    ) -> Result<ProjectionSubscriptionRequest, ProductAdapterError> {
        let authority = self.projection_authority.as_ref().ok_or_else(|| {
            ProductAdapterError::from(ProductWorkflowError::UnsupportedActionKind {
                kind: "projection_subscription".into(),
            })
        })?;

        let (thread_id_hint, after_cursor) = match envelope.payload() {
            ProductInboundPayload::SubscriptionRequest(req) => {
                (req.thread_id_hint.clone(), req.after_cursor.clone())
            }
            _ => (None, None),
        };

        let request = ProjectionSubscriptionAuthorityRequest {
            adapter_id: envelope.adapter_id().clone(),
            installation_id: envelope.installation_id().clone(),
            external_event_id: envelope.external_event_id().clone(),
            external_actor_ref: envelope.external_actor_ref().clone(),
            external_conversation_ref: envelope.external_conversation_ref().clone(),
            thread_id_hint,
            after_cursor,
        };

        authority
            .authorize_subscription(request)
            .await
            .map_err(ProductAdapterError::from)
    }
}

impl DefaultProductWorkflow {
    async fn dispatch_payload(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<ProductInboundAck, ProductWorkflowError> {
        match envelope.payload() {
            ProductInboundPayload::UserMessage(_) => {
                let outcome = self
                    .inbound_turn_service
                    .accept_user_message(envelope)
                    .await?;
                Ok(outcome.to_ack())
            }
            ProductInboundPayload::Command(cmd) => self.dispatch_command(envelope, cmd).await,
            ProductInboundPayload::ApprovalResolution(res) => {
                self.dispatch_approval(envelope, res).await
            }
            ProductInboundPayload::AuthResolution(res) => self.dispatch_auth(envelope, res).await,
            ProductInboundPayload::SubscriptionRequest(_) => {
                // Subscription requests never reach dispatch_payload: accept_inbound
                // short-circuits them above. Defense-in-depth in case the gate moves.
                Err(ProductWorkflowError::UnsupportedActionKind {
                    kind: "subscription_request".into(),
                })
            }
            ProductInboundPayload::LinkedThreadAction(lta) => {
                self.dispatch_linked_thread_action(envelope, lta).await
            }
            ProductInboundPayload::MissionAction(ma) => {
                self.dispatch_mission_action(envelope, ma).await
            }
            ProductInboundPayload::SystemAction(sa) => {
                self.dispatch_system_action(envelope, sa).await
            }
            ProductInboundPayload::NoOp => Ok(ProductInboundAck::NoOp),
        }
    }

    async fn dispatch_command(
        &self,
        envelope: &ProductInboundEnvelope,
        payload: &ironclaw_product_adapters::InboundCommandPayload,
    ) -> Result<ProductInboundAck, ProductWorkflowError> {
        let router = self.command_router.as_ref().ok_or_else(|| {
            ProductWorkflowError::CommandRoutingUnavailable {
                command: payload.command.clone(),
            }
        })?;

        let command = crate::action::ProductCommandName::new(payload.command.clone())
            .map_err(|reason| ProductWorkflowError::TurnSubmissionRejected { reason })?;
        let outcome = router
            .route_command(envelope, command, payload.arguments.clone())
            .await?;

        match outcome {
            ProductCommandOutcome::Routed { command } => {
                let wire_command =
                    AdapterCommandName::new(command.as_str().to_string()).map_err(|err| {
                        ProductWorkflowError::TurnSubmissionRejected {
                            reason: format!("invalid command name for ack: {err}"),
                        }
                    })?;
                Ok(ProductInboundAck::CommandRouted {
                    command: wire_command,
                })
            }
            ProductCommandOutcome::UnknownCommand { command } => {
                Ok(ProductInboundAck::Rejected(ProductRejection::permanent(
                    ProductRejectionKind::PolicyDenied,
                    format!("unknown command: {command}"),
                )))
            }
        }
    }

    async fn dispatch_approval(
        &self,
        envelope: &ProductInboundEnvelope,
        payload: &ironclaw_product_adapters::ApprovalResolutionPayload,
    ) -> Result<ProductInboundAck, ProductWorkflowError> {
        let service = self.approval_service.as_ref().ok_or_else(|| {
            ProductWorkflowError::UnsupportedActionKind {
                kind: "approval_resolution".into(),
            }
        })?;
        let gate_ref = ironclaw_turns::LoopGateRef::new(payload.gate_ref.clone())
            .map_err(|reason| ProductWorkflowError::TurnResumeRejected { reason })?;
        let outcome = service
            .resolve_approval(envelope, gate_ref, payload.decision.clone())
            .await?;
        match outcome {
            ApprovalResolutionOutcome::Handled { gate_ref } => {
                let wire_ref =
                    AdapterLoopGateRef::new(gate_ref.as_str().to_string()).map_err(|err| {
                        ProductWorkflowError::TurnResumeRejected {
                            reason: format!("invalid gate ref for ack: {err}"),
                        }
                    })?;
                Ok(ProductInboundAck::GateHandled { gate_ref: wire_ref })
            }
            ApprovalResolutionOutcome::StaleOrUnknown => {
                Ok(ProductInboundAck::Rejected(ProductRejection::permanent(
                    ProductRejectionKind::PolicyDenied,
                    "approval gate is stale or unknown",
                )))
            }
        }
    }

    async fn dispatch_auth(
        &self,
        envelope: &ProductInboundEnvelope,
        payload: &ironclaw_product_adapters::AuthResolutionPayload,
    ) -> Result<ProductInboundAck, ProductWorkflowError> {
        let service = self.auth_service.as_ref().ok_or_else(|| {
            ProductWorkflowError::UnsupportedActionKind {
                kind: "auth_resolution".into(),
            }
        })?;
        let auth_request_ref = crate::action::AuthRequestRef::new(payload.auth_request_ref.clone())
            .map_err(|reason| ProductWorkflowError::TurnResumeRejected { reason })?;
        let outcome = service
            .resolve_auth(envelope, auth_request_ref, payload.result.clone())
            .await?;
        match outcome {
            AuthResolutionOutcome::Handled { auth_request_ref } => {
                // Project auth_request_ref into the wire-stable LoopGateRef
                // wrapper used by the GateHandled ack — both approval and auth
                // resolutions surface through the same "gate handled" channel.
                let wire_ref = AdapterLoopGateRef::new(auth_request_ref.as_str().to_string())
                    .map_err(|err| ProductWorkflowError::TurnResumeRejected {
                        reason: format!("invalid auth request ref for ack: {err}"),
                    })?;
                Ok(ProductInboundAck::GateHandled { gate_ref: wire_ref })
            }
            AuthResolutionOutcome::StaleOrUnknown => {
                Ok(ProductInboundAck::Rejected(ProductRejection::permanent(
                    ProductRejectionKind::PolicyDenied,
                    "auth request is stale or unknown",
                )))
            }
        }
    }

    async fn dispatch_linked_thread_action(
        &self,
        envelope: &ProductInboundEnvelope,
        payload: &ironclaw_product_adapters::LinkedThreadActionPayload,
    ) -> Result<ProductInboundAck, ProductWorkflowError> {
        let service = self.linked_thread_service.as_ref().ok_or_else(|| {
            ProductWorkflowError::UnsupportedActionKind {
                kind: "linked_thread_action".into(),
            }
        })?;
        let action_id = crate::action::LinkedThreadActionId::new(payload.action_id.clone())
            .map_err(|reason| ProductWorkflowError::TurnSubmissionRejected { reason })?;
        let outcome = service
            .handle_action(
                envelope,
                action_id,
                payload.data.clone(),
                payload.reply_target_message_id.clone(),
            )
            .await?;
        match outcome {
            LinkedThreadActionOutcome::Routed { action_id } => {
                let wire_id = ironclaw_product_adapters::LinkedThreadActionId::new(
                    action_id.as_str().to_string(),
                )
                .map_err(|err| ProductWorkflowError::TurnSubmissionRejected {
                    reason: format!("invalid linked thread action id for ack: {err}"),
                })?;
                Ok(ProductInboundAck::LinkedThreadActionRouted { action_id: wire_id })
            }
        }
    }

    async fn dispatch_mission_action(
        &self,
        envelope: &ProductInboundEnvelope,
        payload: &ironclaw_product_adapters::MissionActionPayload,
    ) -> Result<ProductInboundAck, ProductWorkflowError> {
        let service = self.mission_service.as_ref().ok_or_else(|| {
            ProductWorkflowError::UnsupportedActionKind {
                kind: "mission_action".into(),
            }
        })?;
        let request = MissionFireRequest {
            adapter_id: envelope.adapter_id().clone(),
            installation_id: envelope.installation_id().clone(),
            external_event_id: envelope.external_event_id().clone(),
            external_actor_ref: envelope.external_actor_ref().clone(),
            external_conversation_ref: envelope.external_conversation_ref().clone(),
            received_at: envelope.received_at(),
            mission_intent: payload.mission_intent.clone(),
            mission_id_hint: payload.mission_id_hint.clone(),
            data: payload.data.clone(),
        };
        let outcome = service.fire_mission(request).await?;
        match outcome {
            MissionFireOutcome::Submitted {
                mission_fire_ref,
                run_id,
            } => Ok(ProductInboundAck::MissionSubmitted {
                mission_fire_ref: ironclaw_product_adapters::MissionFireRef::from_uuid(
                    mission_fire_ref.as_uuid(),
                ),
                submitted_run_id: run_id,
            }),
            MissionFireOutcome::DeferredBusy {
                mission_fire_ref, ..
            } => Ok(ProductInboundAck::MissionSuppressed {
                mission_fire_ref: ironclaw_product_adapters::MissionFireRef::from_uuid(
                    mission_fire_ref.as_uuid(),
                ),
                reason: ironclaw_product_adapters::MissionFireSuppressionReason::BusyThread,
            }),
            MissionFireOutcome::Suppressed {
                mission_fire_ref,
                reason,
            } => Ok(ProductInboundAck::MissionSuppressed {
                mission_fire_ref: ironclaw_product_adapters::MissionFireRef::from_uuid(
                    mission_fire_ref.as_uuid(),
                ),
                reason: workflow_to_wire_suppression(reason),
            }),
            MissionFireOutcome::Rejected { reason } => {
                Ok(ProductInboundAck::Rejected(ProductRejection::permanent(
                    ProductRejectionKind::PolicyDenied,
                    format!("mission fire rejected: {reason:?}"),
                )))
            }
        }
    }

    async fn dispatch_system_action(
        &self,
        envelope: &ProductInboundEnvelope,
        payload: &ironclaw_product_adapters::SystemActionPayload,
    ) -> Result<ProductInboundAck, ProductWorkflowError> {
        let service = self.system_action_service.as_ref().ok_or_else(|| {
            ProductWorkflowError::UnsupportedActionKind {
                kind: "system_action".into(),
            }
        })?;
        let _outcome = service
            .handle_action(
                envelope,
                payload.system_actor_ref.clone(),
                payload.kind.clone(),
                payload.scope_thread_id.clone(),
                payload.data.clone(),
            )
            .await?;
        // System actions are durable but ack-less in this slice; return NoOp
        // so adapters know the action settled without exposing internal state.
        Ok(ProductInboundAck::NoOp)
    }
}

fn workflow_to_wire_suppression(
    reason: WorkflowMissionFireSuppressionReason,
) -> ironclaw_product_adapters::MissionFireSuppressionReason {
    match reason {
        WorkflowMissionFireSuppressionReason::Cadence => {
            ironclaw_product_adapters::MissionFireSuppressionReason::Cadence
        }
        WorkflowMissionFireSuppressionReason::Cooldown => {
            ironclaw_product_adapters::MissionFireSuppressionReason::Cooldown
        }
        WorkflowMissionFireSuppressionReason::Deduplicated => {
            ironclaw_product_adapters::MissionFireSuppressionReason::Deduplicated
        }
        WorkflowMissionFireSuppressionReason::BusyThread => {
            ironclaw_product_adapters::MissionFireSuppressionReason::BusyThread
        }
    }
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
        ProductInboundAck::MissionSubmitted {
            submitted_run_id, ..
        } => Ok(ActionDispatchKind::UserMessageTurn {
            run_id: *submitted_run_id,
        }),
        _ => ActionDispatchKind::try_from_payload(payload),
    }
}

fn is_terminal_success_ack(ack: &ProductInboundAck) -> bool {
    !matches!(ack, ProductInboundAck::DeferredBusy { .. })
}

fn turn_error_is_retryable(error: &TurnError) -> bool {
    matches!(error.adapter_status_code(), 429 | 503)
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
        | TurnErrorCategory::Unavailable
        | TurnErrorCategory::Conflict => ProductRejectionKind::PolicyDenied,
    }
}

fn terminal_ack_for_error(error: &ProductWorkflowError) -> Option<ProductInboundAck> {
    match error {
        ProductWorkflowError::CommandRoutingUnavailable { command } => {
            Some(ProductInboundAck::Rejected(ProductRejection::permanent(
                ProductRejectionKind::PolicyDenied,
                format!("command routing unavailable: {command}"),
            )))
        }
        ProductWorkflowError::UnsupportedActionKind { kind } => {
            Some(ProductInboundAck::Rejected(ProductRejection::permanent(
                ProductRejectionKind::PolicyDenied,
                format!("unsupported action kind: {kind}"),
            )))
        }
        ProductWorkflowError::TurnSubmissionFailed { error } if !turn_error_is_retryable(error) => {
            Some(ProductInboundAck::Rejected(ProductRejection::permanent(
                rejection_kind_for_turn_error(error),
                format!("turn submission rejected: {error}"),
            )))
        }
        ProductWorkflowError::TurnResumeRejected { reason } => {
            Some(ProductInboundAck::Rejected(ProductRejection::permanent(
                ProductRejectionKind::PolicyDenied,
                format!("turn resume rejected: {reason}"),
            )))
        }
        ProductWorkflowError::BindingResolutionFailed { .. }
        | ProductWorkflowError::TurnSubmissionRejected { .. }
        | ProductWorkflowError::TurnSubmissionFailed { .. }
        | ProductWorkflowError::Transient { .. }
        | ProductWorkflowError::DuplicateAction { .. } => None,
    }
}
