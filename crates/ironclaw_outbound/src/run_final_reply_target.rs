use async_trait::async_trait;
use ironclaw_host_api::{ResourceScope, RunId, UserId};
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};
use serde::{Deserialize, Serialize};

use crate::OutboundDeliveryTargetId;

/// Stable id for the host-owned WebApp final-reply destination.
pub const WEB_APP_OUTBOUND_DELIVERY_TARGET_ID: &str = "builtin:web_app";

/// The host-sealed destination for the final reply of one run.
///
/// This is metadata only. It never contains message content or provider
/// credentials. External destinations remain subject to current target
/// revalidation immediately before delivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum RunFinalReplyDestination {
    /// Keep the completed reply in the durable transcript without external
    /// channel delivery.
    WebApp,
    /// Deliver through the currently-authorized provider target represented by
    /// this opaque binding reference.
    External {
        reply_target_binding_ref: ReplyTargetBindingRef,
    },
}

/// Durable per-run final-reply routing decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunFinalReplyTargetRecord {
    pub run_id: TurnRunId,
    pub scope: TurnScope,
    pub actor: TurnActor,
    pub destination: RunFinalReplyDestination,
}

/// Exact authority tuple required to read a per-run routing decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunFinalReplyTargetRequest {
    pub run_id: TurnRunId,
    pub scope: TurnScope,
    pub actor: TurnActor,
}

/// Trusted request passed from the normal first-party capability lane to the
/// product-owned outbound routing service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteCurrentRunFinalReplyRequest {
    pub run_id: RunId,
    pub scope: ResourceScope,
    pub authenticated_actor_user_id: UserId,
    pub target_id: OutboundDeliveryTargetId,
}

/// Redacted failure classes returned across the capability-to-product port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RouteCurrentRunFinalReplyError {
    #[error("current-run final-reply route is invalid")]
    InvalidRequest,
    #[error("current-run final-reply route is not permitted")]
    AccessDenied,
    #[error("outbound delivery target is unavailable")]
    TargetUnavailable,
    #[error("outbound delivery routing service is unavailable")]
    Unavailable,
    #[error("outbound delivery routing failed")]
    Internal,
}

/// Product-owned mutation port consumed by the normal first-party capability
/// handler. Implementations must resolve `target_id` through current
/// caller-scoped target authority before persisting the per-run destination.
#[async_trait]
pub trait RouteCurrentRunFinalReply: Send + Sync {
    async fn route_current_run_final_reply(
        &self,
        request: RouteCurrentRunFinalReplyRequest,
    ) -> Result<(), RouteCurrentRunFinalReplyError>;
}

#[cfg(test)]
mod tests {
    use super::OutboundDeliveryTargetId;

    #[test]
    fn target_id_rejects_invisible_formatting_characters() {
        let error = OutboundDeliveryTargetId::new("target:\u{200b}hidden")
            .expect_err("invisible formatting must be rejected");
        assert!(error.contains("unsafe Unicode formatting"));
    }

    #[test]
    fn outbound_reexport_keeps_neutral_serde_and_length_contract() {
        let value = format!("provider:{}", "x".repeat(503));
        let target = OutboundDeliveryTargetId::new(value.clone()).expect("512-byte target");
        let encoded = serde_json::to_value(&target).expect("serialize target");
        assert_eq!(
            serde_json::from_value::<OutboundDeliveryTargetId>(encoded)
                .expect("deserialize target"),
            target
        );
        assert_eq!(target.as_str(), value);
        assert!(OutboundDeliveryTargetId::new("x".repeat(513)).is_err());
    }
}
