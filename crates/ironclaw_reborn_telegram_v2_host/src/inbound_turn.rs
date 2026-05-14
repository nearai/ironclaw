//! Stubbed [`InboundTurnService`] for the standalone host.
//!
//! Does the smallest honest thing: resolve the binding (creating the
//! `(tenant, user, thread)` row on first inbound) and return
//! `InboundTurnOutcome::Submitted` with a synthetic correlation id. **No
//! reply is produced.** The webhook handler will then ack 200 to Telegram
//! and the message is persisted in the ledger; nothing else happens until
//! the Reborn agent loop ships (PRs #3544 / #3550 / #3586).
//!
//! When the Reborn loop lands:
//!   * Drop this file.
//!   * Wire `DefaultInboundTurnService` (from `ironclaw_product_workflow`)
//!     + a real `TurnCoordinator` instead.
//!   * The webhook router, composition, migrations, and storage layer all
//!     stay the same.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_product_adapters::{ProductInboundEnvelope, ProductInboundPayload};
use ironclaw_product_workflow::{
    ConversationBindingService, InboundTurnOutcome, InboundTurnService, ProductWorkflowError,
    ResolveBindingRequest,
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
            "Reborn host: inbound resolved + bound; reply path stubbed (no Reborn agent loop yet)"
        );

        Ok(InboundTurnOutcome::Submitted {
            accepted_message_ref: accepted_ref,
            submitted_run_id: synthetic_run_id,
            binding,
        })
    }
}
