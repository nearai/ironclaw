//! Workflow-owned command admission and execution ports.
//!
//! `commands` owns the source-agnostic command model. This module owns the
//! authority-bearing workflow context and the service boundary that decides
//! whether a command may execute from that context.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_product_adapters::{
    AdapterInstallationId, ExternalActorRef, ExternalConversationRef, ProductAdapterId,
    ProductInboundAck, ProductInboundEnvelope, ProductInboundPayload, ProductRejection,
    ProductRejectionKind, ProductTriggerReason, VerifiedAuthClaim,
};
use serde::{Deserialize, Serialize};

use crate::commands::ProductCommand;
use crate::error::ProductWorkflowError;

/// Authority-bearing command dispatch context built by the workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProductCommandContext {
    pub adapter_id: ProductAdapterId,
    pub installation_id: AdapterInstallationId,
    pub external_actor_ref: ExternalActorRef,
    pub external_conversation_ref: ExternalConversationRef,
    pub auth_claim: VerifiedAuthClaim,
    pub trigger: ProductTriggerReason,
    pub received_at: DateTime<Utc>,
}

impl ProductCommandContext {
    pub fn from_envelope(envelope: &ProductInboundEnvelope) -> Result<Self, ProductWorkflowError> {
        let ProductInboundPayload::Command(command) = envelope.payload() else {
            return Err(ProductWorkflowError::UnsupportedActionKind {
                kind: "non_command".to_string(),
            });
        };
        Ok(Self {
            adapter_id: envelope.adapter_id().clone(),
            installation_id: envelope.installation_id().clone(),
            external_actor_ref: envelope.external_actor_ref().clone(),
            external_conversation_ref: envelope.external_conversation_ref().clone(),
            auth_claim: envelope.auth_claim().clone(),
            trigger: command.trigger,
            received_at: envelope.received_at(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProductCommandAdmission {
    Allowed,
    Rejected(ProductRejection),
}

#[async_trait]
pub trait ProductCommandAdmissionService: Send + Sync {
    async fn admit(
        &self,
        context: &ProductCommandContext,
        command: &ProductCommand,
    ) -> Result<ProductCommandAdmission, ProductWorkflowError>;
}

/// Fail-closed admission service used until a host composition supplies concrete
/// source/auth policy.
pub struct RejectingProductCommandAdmissionService;

#[async_trait]
impl ProductCommandAdmissionService for RejectingProductCommandAdmissionService {
    async fn admit(
        &self,
        _context: &ProductCommandContext,
        command: &ProductCommand,
    ) -> Result<ProductCommandAdmission, ProductWorkflowError> {
        Ok(ProductCommandAdmission::Rejected(
            ProductRejection::permanent(
                ProductRejectionKind::PolicyDenied,
                format!("command routing unavailable: {}", command.name()),
            ),
        ))
    }
}

#[async_trait]
pub trait ProductCommandService: Send + Sync {
    async fn execute(
        &self,
        context: ProductCommandContext,
        command: ProductCommand,
    ) -> Result<ProductInboundAck, ProductWorkflowError>;
}

/// Fail-closed command executor used until a host composition supplies concrete
/// command implementations.
pub struct RejectingProductCommandService;

#[async_trait]
impl ProductCommandService for RejectingProductCommandService {
    async fn execute(
        &self,
        _context: ProductCommandContext,
        command: ProductCommand,
    ) -> Result<ProductInboundAck, ProductWorkflowError> {
        Ok(ProductInboundAck::Rejected(ProductRejection::permanent(
            ProductRejectionKind::PolicyDenied,
            format!("command routing unavailable: {}", command.name()),
        )))
    }
}
