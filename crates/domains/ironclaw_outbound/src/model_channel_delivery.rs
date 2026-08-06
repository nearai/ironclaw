use async_trait::async_trait;
use ironclaw_host_api::ids::{RunId, UserId};
use ironclaw_host_api::resource::ResourceScope;

use crate::{DeliveryFailureKind, OutboundDeliveryTargetId, OutboundDeliveryTargetSummary};

/// Per-run ceiling on explicit model deliveries (spec §5).
pub const MODEL_DELIVERY_PER_RUN_CAP: u32 = 16;
/// Input content ceiling, matched by the tool input schema (spec §5).
pub const MODEL_DELIVERY_MAX_CONTENT_BYTES: usize = 32_768;

/// Trusted request from the first-party capability lane: the host, not the
/// model, supplies `scope`, `run_id`, and `authenticated_actor_user_id`; only
/// `target_id` and the content come from model-authored input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelChannelDeliveryRequest {
    pub scope: ResourceScope,
    pub run_id: RunId,
    pub authenticated_actor_user_id: UserId,
    pub target_id: OutboundDeliveryTargetId,
    pub content: String,
}

/// Provider-issued evidence for one delivered call (spec §5: never claim
/// what the vendor did not confirm).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelChannelDeliveryEvidence {
    pub target: OutboundDeliveryTargetSummary,
    pub provider_message_refs: Vec<String>,
    /// `true` when the durable `Delivered` confirmation row committed.
    /// `false` is the explicit weaker evidence `tool-evidence.md` requires
    /// when the provider accepted the send but the confirmation write failed
    /// (`CoordinatedDeliveryOutcome::DeliveredUnconfirmed`): the refs are
    /// real, the message must not be resent, and recovery reconciles the row.
    pub durably_recorded: bool,
}

/// Redacted failure classes crossing the capability-to-product port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ModelChannelDeliveryError {
    #[error("outbound delivery target is unavailable")]
    TargetUnavailable,
    #[error("target is this run's own origin conversation")]
    OriginConversationTarget,
    #[error("per-run delivery cap reached")]
    DeliveryCapExceeded,
    #[error("delivery content exceeds the size bound")]
    ContentTooLarge,
    #[error("outbound policy rejected the delivery")]
    Rejected,
    #[error("delivery failed")]
    Failed { kind: DeliveryFailureKind },
    #[error("delivery is not permitted")]
    AccessDenied,
    #[error("delivery service is unavailable")]
    Unavailable,
    #[error("delivery failed internally")]
    Internal,
}

#[async_trait]
pub trait ModelChannelDelivery: Send + Sync {
    async fn deliver_for_model(
        &self,
        request: ModelChannelDeliveryRequest,
    ) -> Result<ModelChannelDeliveryEvidence, ModelChannelDeliveryError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_error_displays_with_failure_kind() {
        let error = ModelChannelDeliveryError::Failed {
            kind: DeliveryFailureKind::AuthorizationRevoked,
        };
        let display_str = error.to_string();
        assert!(display_str.contains("delivery failed"));
    }
}
