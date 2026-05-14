//! Host-side `ProductWorkflow` implementation.
//!
//! This is the top-level product action orchestrator that dispatches inbound
//! envelopes to the appropriate downstream service based on payload kind.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_product_adapters::{
    ProductAdapterError, ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload,
    ProductRejection, ProductRejectionKind, ProductWorkflow, ProjectionSubscriptionRequest,
};
use ironclaw_turns::{AdmissionRejectionReason, TurnError, TurnErrorCategory};
use tracing::debug;

use crate::action::{ActionDispatchKind, ActionFingerprintKey, SourceBindingKey};
use crate::error::ProductWorkflowError;
use crate::inbound_turn::InboundTurnService;
use crate::ledger::{IdempotencyDecision, IdempotencyLedger};
use crate::policy::{
    BeforeInboundPolicy, BeforeInboundPolicyOutcome, BeforeInboundPolicyRequest,
    NoopBeforeInboundPolicy,
};

/// Host-side implementation of [`ProductWorkflow`] that dispatches inbound
/// envelopes through the idempotency ledger and routes to the appropriate
/// downstream service.
pub struct DefaultProductWorkflow {
    inbound_turn_service: Arc<dyn InboundTurnService>,
    idempotency_ledger: Arc<dyn IdempotencyLedger>,
    before_inbound_policy: Arc<dyn BeforeInboundPolicy>,
}

impl DefaultProductWorkflow {
    pub fn new(
        inbound_turn_service: Arc<dyn InboundTurnService>,
        idempotency_ledger: Arc<dyn IdempotencyLedger>,
    ) -> Self {
        Self {
            inbound_turn_service,
            idempotency_ledger,
            before_inbound_policy: Arc::new(NoopBeforeInboundPolicy),
        }
    }

    pub fn with_before_inbound_policy(
        mut self,
        before_inbound_policy: Arc<dyn BeforeInboundPolicy>,
    ) -> Self {
        self.before_inbound_policy = before_inbound_policy;
        self
    }
}

#[async_trait]
impl ProductWorkflow for DefaultProductWorkflow {
    async fn accept_inbound(
        &self,
        envelope: ProductInboundEnvelope,
    ) -> Result<ProductInboundAck, ProductAdapterError> {
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
                    detail: ironclaw_product_adapters::RedactedString::new(
                        "settled action missing outcome",
                    ),
                })
            }
            IdempotencyDecision::New(mut action) => {
                let result = dispatch_payload(
                    &envelope,
                    &*self.inbound_turn_service,
                    &*self.before_inbound_policy,
                )
                .await;

                match result {
                    Ok(dispatched) => {
                        action.mark_dispatched(dispatched.dispatch_kind);
                        if should_settle_ack(&dispatched.ack) {
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
        _envelope: ProductInboundEnvelope,
    ) -> Result<ProjectionSubscriptionRequest, ProductAdapterError> {
        Err(ProductAdapterError::Internal {
            detail: ironclaw_product_adapters::RedactedString::new(
                "projection subscription resolution not yet implemented",
            ),
        })
    }
}

struct DispatchedAction {
    ack: ProductInboundAck,
    dispatch_kind: ActionDispatchKind,
}

async fn dispatch_payload(
    envelope: &ProductInboundEnvelope,
    inbound_turn_service: &dyn InboundTurnService,
    before_inbound_policy: &dyn BeforeInboundPolicy,
) -> Result<DispatchedAction, ProductWorkflowError> {
    match envelope.payload() {
        ProductInboundPayload::UserMessage(payload) => {
            let policy_request = BeforeInboundPolicyRequest::new(envelope, payload);
            let policy_outcome = before_inbound_policy
                .check_user_message(policy_request)
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
                    let dispatch_kind = ActionDispatchKind::Rejected {
                        kind: rejection.kind.clone(),
                    };
                    debug!(
                        rejection_kind = ?rejection.kind,
                        disposition = ?rejection.disposition(),
                        "before-inbound policy rejected user message"
                    );
                    return Ok(DispatchedAction {
                        ack: ProductInboundAck::Rejected(rejection),
                        dispatch_kind,
                    });
                }
            };
            let outcome = inbound_turn_service
                .accept_user_message(envelope_for_turn)
                .await?;
            let ack = outcome.to_ack();
            let dispatch_kind = dispatch_kind_from_ack(&ack, envelope_for_turn.payload())?;
            Ok(DispatchedAction { ack, dispatch_kind })
        }
        ProductInboundPayload::Command(cmd) => {
            Err(ProductWorkflowError::CommandRoutingUnavailable {
                command: cmd.command.clone(),
            })
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
        _ => ActionDispatchKind::try_from_payload(payload),
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
        ProductWorkflowError::BindingResolutionFailed { .. }
        | ProductWorkflowError::TurnSubmissionRejected { .. }
        | ProductWorkflowError::TurnSubmissionFailed { .. }
        | ProductWorkflowError::TurnResumeRejected { .. }
        | ProductWorkflowError::Transient { .. }
        | ProductWorkflowError::DuplicateAction { .. } => None,
    }
}
