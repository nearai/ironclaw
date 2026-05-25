//! Host-side `ProductWorkflow` implementation.
//!
//! This is the top-level product action orchestrator that dispatches inbound
//! envelopes to the appropriate downstream service based on payload kind.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_product_adapters::{
    ProductAdapterError, ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload,
    ProductRejection, ProductRejectionKind, ProductTriggerReason, ProductWorkflow,
    ProductWorkflowRejectionKind, ProjectionSubscriptionRequest, RedactedString,
};
use ironclaw_turns::{
    AdmissionRejectionReason, TurnActor, TurnError, TurnErrorCategory, TurnScope,
};
use tracing::debug;

use crate::action::{
    ActionDispatchKind, ActionFingerprintKey, ProductCommandName, SourceBindingKey,
};
use crate::binding::{
    ConversationBindingService, ProductConversationRouteKind, ResolveBindingRequest,
    ResolvedBinding,
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
}

#[async_trait]
impl ProductWorkflow for DefaultProductWorkflow {
    async fn accept_inbound(
        &self,
        envelope: ProductInboundEnvelope,
    ) -> Result<ProductInboundAck, ProductAdapterError> {
        if matches!(
            envelope.payload(),
            ProductInboundPayload::SubscriptionRequest(_)
        ) {
            return Err(ProductAdapterError::MalformedInboundPayload {
                reason: RedactedString::new(
                    "subscription_request must be resolved through resolve_projection_subscription",
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
                    &*self.inbound_turn_service,
                    &*self.before_inbound_policy,
                    &*self.command_admission_service,
                    &*self.command_service,
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

    async fn resolve_projection_subscription(
        &self,
        envelope: ProductInboundEnvelope,
    ) -> Result<ProjectionSubscriptionRequest, ProductAdapterError> {
        let ProductInboundPayload::SubscriptionRequest(payload) = envelope.payload() else {
            return Err(ProductAdapterError::MalformedInboundPayload {
                reason: RedactedString::new(
                    "projection subscription resolution requires subscription_request payload",
                ),
            });
        };
        let binding = self
            .binding_service
            .lookup_binding(resolve_binding_request(&envelope))
            .await
            .map_err(ProductAdapterError::from)?;
        let thread_id =
            projection_thread_id_from_binding(&binding, payload.thread_id_hint.as_deref())?;

        Ok(ProjectionSubscriptionRequest {
            actor: TurnActor::new(binding.user_id.clone()),
            scope: TurnScope::new(
                binding.tenant_id.clone(),
                binding.agent_id.clone(),
                binding.project_id.clone(),
                thread_id,
            ),
            after_cursor: payload.after_cursor.clone(),
        })
    }
}

struct DispatchedAction {
    ack: ProductInboundAck,
    dispatch_kind: ActionDispatchKind,
}

fn resolve_binding_request(envelope: &ProductInboundEnvelope) -> ResolveBindingRequest {
    ResolveBindingRequest {
        adapter_id: envelope.adapter_id().clone(),
        installation_id: envelope.installation_id().clone(),
        external_actor_ref: envelope.external_actor_ref().clone(),
        external_conversation_ref: envelope.external_conversation_ref().clone(),
        external_event_id: envelope.external_event_id().clone(),
        route_kind: route_kind_for_payload(envelope.payload()),
        auth_claim: envelope.auth_claim().clone(),
    }
}

fn route_kind_for_payload(payload: &ProductInboundPayload) -> ProductConversationRouteKind {
    match payload {
        ProductInboundPayload::UserMessage(message) => match message.trigger {
            ProductTriggerReason::DirectChat => ProductConversationRouteKind::Direct,
            ProductTriggerReason::BotMention
            | ProductTriggerReason::ReplyToBot
            | ProductTriggerReason::BotCommand
            | ProductTriggerReason::LinkedThreadAction => ProductConversationRouteKind::Shared,
        },
        ProductInboundPayload::Command(command) => match command.trigger {
            ProductTriggerReason::DirectChat => ProductConversationRouteKind::Direct,
            ProductTriggerReason::BotMention
            | ProductTriggerReason::ReplyToBot
            | ProductTriggerReason::BotCommand
            | ProductTriggerReason::LinkedThreadAction => ProductConversationRouteKind::Shared,
        },
        _ => ProductConversationRouteKind::Direct,
    }
}

fn projection_thread_id_from_binding(
    binding: &ResolvedBinding,
    thread_id_hint: Option<&str>,
) -> Result<ironclaw_host_api::ThreadId, ProductAdapterError> {
    if let Some(thread_id_hint) = thread_id_hint {
        let hinted = ironclaw_host_api::ThreadId::new(thread_id_hint).map_err(|_| {
            ProductAdapterError::MalformedInboundPayload {
                reason: RedactedString::new("invalid thread_id_hint"),
            }
        })?;
        if hinted != binding.thread_id {
            return Err(ProductAdapterError::WorkflowRejected {
                kind: ProductWorkflowRejectionKind::InvalidRequest,
                status_code: 400,
                retryable: false,
                reason: RedactedString::new(
                    "thread_id_hint does not match resolved conversation binding",
                ),
            });
        }
    }
    Ok(binding.thread_id.clone())
}

async fn dispatch_payload(
    envelope: &ProductInboundEnvelope,
    action_id: crate::ProductActionId,
    action_fingerprint: ActionFingerprintKey,
    inbound_turn_service: &dyn InboundTurnService,
    before_inbound_policy: &dyn BeforeInboundPolicy,
    command_admission_service: &dyn ProductCommandAdmissionService,
    command_service: &dyn ProductCommandService,
) -> Result<DispatchedAction, ProductWorkflowError> {
    match envelope.payload() {
        ProductInboundPayload::UserMessage(_) => {
            match inbound_turn_service
                .accept_user_message_with_before_policy(envelope, before_inbound_policy)
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
            let command = ProductCommand::from_payload(cmd);
            match command_admission_service.admit(&context, &command).await? {
                ProductCommandAdmission::Allowed => {}
                ProductCommandAdmission::Rejected(rejection) => {
                    let ack = ProductInboundAck::Rejected(rejection);
                    let dispatch_kind = dispatch_kind_from_ack(&ack, envelope.payload())?;
                    return Ok(DispatchedAction { ack, dispatch_kind });
                }
            }
            let ack = command_service.execute(context, command).await?;
            let dispatch_kind = dispatch_kind_from_command_ack(&ack)?;
            Ok(DispatchedAction { ack, dispatch_kind })
        }
        ProductInboundPayload::ApprovalResolution(_) => {
            Err(ProductWorkflowError::UnsupportedActionKind {
                kind: "approval_resolution".into(),
            })
        }
        ProductInboundPayload::AuthResolution(_) => {
            Err(ProductWorkflowError::UnsupportedActionKind {
                kind: "auth_resolution".into(),
            })
        }
        ProductInboundPayload::SubscriptionRequest(_) => {
            Err(ProductWorkflowError::UnsupportedActionKind {
                kind: "subscription_request".into(),
            })
        }
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
) -> Result<ActionDispatchKind, ProductWorkflowError> {
    match ack {
        ProductInboundAck::CommandRouted { command } => Ok(ActionDispatchKind::Command {
            command: ProductCommandName::new(command.as_str())
                .map_err(|reason| ProductWorkflowError::TurnSubmissionRejected { reason })?,
        }),
        ProductInboundAck::Rejected(rejection) => Ok(ActionDispatchKind::Rejected {
            kind: rejection.kind.clone(),
        }),
        ProductInboundAck::Accepted { .. }
        | ProductInboundAck::DeferredBusy { .. }
        | ProductInboundAck::Duplicate { .. }
        | ProductInboundAck::NoOp => Err(ProductWorkflowError::UnsupportedActionKind {
            kind: "non_command_ack_from_product_command".into(),
        }),
    }
}

fn should_settle_ack(ack: &ProductInboundAck) -> bool {
    !matches!(ack, ProductInboundAck::DeferredBusy { .. }) && ack.is_durable_outcome()
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
        | ProductWorkflowError::TurnResumeDenied { .. }
        | ProductWorkflowError::Transient { .. }
        | ProductWorkflowError::BeforeInboundPolicyFailed {
            permanent: false, ..
        }
        | ProductWorkflowError::DuplicateAction { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_product_adapters::{ProductInboundAck, ProductInboundPayload};
    use ironclaw_turns::{AcceptedMessageRef, AdmissionRejection, TurnRunId};

    use super::*;

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
