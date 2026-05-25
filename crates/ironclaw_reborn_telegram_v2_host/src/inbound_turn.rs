//! Stubbed [`InboundTurnService`] for the standalone host.
//!
//! Does the smallest honest thing: resolve the binding (creating the
//! `(tenant, user, thread)` row on first inbound) and return
//! `InboundTurnOutcome::Submitted` with a synthetic correlation id. **No
//! reply is produced.** The webhook handler will then ack 200 to Telegram
//! and the message is persisted in the ledger.
//!
//! The Reborn agent loop (PRs #3544 / #3550 / #3586) has now merged, but
//! this PR is the inbound tracer — scoped to locking down the inbound
//! contract before wiring the outbound reply path. The migration is a
//! deliberate follow-up:
//!   * Drop this file.
//!   * Wire `DefaultInboundTurnService` (from `ironclaw_product_workflow`)
//!     + a real `TurnCoordinator` instead.
//!   * The webhook router, composition, migrations, and storage layer all
//!     stay the same.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_product_adapters::{ProductInboundEnvelope, ProductInboundPayload};
use ironclaw_product_workflow::{
    BeforeInboundPolicy, BeforeInboundPolicyOutcome, BeforeInboundPolicyRequest,
    ConversationBindingService, InboundTurnOutcome, InboundTurnService, InboundUserMessageDispatch,
    ProductWorkflowError, ResolveBindingRequest,
};
use ironclaw_turns::{AcceptedMessageRef, TurnRunId};

pub struct StubInboundTurnService {
    binding: Arc<dyn ConversationBindingService>,
}

impl StubInboundTurnService {
    pub fn new(binding: Arc<dyn ConversationBindingService>) -> Self {
        Self { binding }
    }
}

#[async_trait]
impl InboundTurnService for StubInboundTurnService {
    /// Stub-only replay probe: this host never persists an accepted-message
    /// projection (no `SessionThreadService` wired), so there is nothing to
    /// replay. Returning `Ok(None)` keeps the workflow on the fresh-accept
    /// path on every inbound. The real `DefaultInboundTurnService` consults
    /// the session-thread store; the outbound migration follow-up swaps this
    /// stub for the real service.
    async fn replay_accepted_user_message(
        &self,
        _envelope: &ProductInboundEnvelope,
    ) -> Result<Option<InboundTurnOutcome>, ProductWorkflowError> {
        Ok(None)
    }

    /// Apply the before-inbound policy then defer to `accept_user_message`.
    /// The stub has no async pre-state to consult, so the policy gate is the
    /// only thing the dispatch path adds on top of `accept_user_message`.
    async fn accept_user_message_with_before_policy(
        &self,
        envelope: &ProductInboundEnvelope,
        before_inbound_policy: &dyn BeforeInboundPolicy,
    ) -> Result<InboundUserMessageDispatch, ProductWorkflowError> {
        let ProductInboundPayload::UserMessage(payload) = envelope.payload() else {
            return Err(ProductWorkflowError::UnsupportedActionKind {
                kind: format!("{:?}", envelope.payload()),
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
            // `BeforeInboundPolicyOutcome` is `#[non_exhaustive]`; any new
            // variant in `ironclaw_product_workflow` should be picked up
            // explicitly during the outbound migration follow-up rather than
            // silently routed to allow/reject in this stub.
            _ => {
                return Err(ProductWorkflowError::TurnSubmissionRejected {
                    reason: "unsupported before-inbound policy outcome variant in tracer stub"
                        .into(),
                });
            }
        };

        self.accept_user_message(envelope_for_turn)
            .await
            .map(InboundUserMessageDispatch::Accepted)
    }

    async fn accept_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<InboundTurnOutcome, ProductWorkflowError> {
        // Only user messages take the binding path. Other action kinds
        // currently aren't reachable through Telegram v2's parse_inbound,
        // but we mirror the trait contract.
        if !matches!(envelope.payload(), ProductInboundPayload::UserMessage(_)) {
            return Err(ProductWorkflowError::UnsupportedActionKind {
                kind: format!("{:?}", envelope.payload()),
            });
        }

        let resolve_request = ResolveBindingRequest {
            adapter_id: envelope.adapter_id().clone(),
            installation_id: envelope.installation_id().clone(),
            external_actor_ref: envelope.external_actor_ref().clone(),
            external_conversation_ref: envelope.external_conversation_ref().clone(),
            external_event_id: envelope.external_event_id().clone(),
            // Stub path: classify Telegram inbound as Direct. The real
            // `DefaultInboundTurnService` derives `route_kind` from
            // `payload.trigger` via `route_kind_for_user_message`; the
            // outbound migration follow-up will replace this stub.
            route_kind: ironclaw_product_workflow::ProductConversationRouteKind::Direct,
            auth_claim: envelope.auth_claim().clone(),
        };
        let binding = self.binding.resolve_binding(resolve_request).await?;

        let synthetic_run_id = TurnRunId::new();
        let accepted_ref = AcceptedMessageRef::new(format!("v2:{}", synthetic_run_id.as_uuid()))
            .map_err(|e| ProductWorkflowError::TurnSubmissionRejected {
                reason: format!("synthesize accepted_message_ref: {e}"),
            })?;

        tracing::info!(
            user_id = %binding.user_id.as_str(),
            thread_id = %binding.thread_id.as_str(),
            run_id = %synthetic_run_id.as_uuid(),
            "Reborn host: inbound resolved + bound; reply path stubbed pending outbound migration"
        );

        Ok(InboundTurnOutcome::Submitted {
            accepted_message_ref: accepted_ref,
            submitted_run_id: synthetic_run_id,
            binding,
        })
    }
}
