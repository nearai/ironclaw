use ironclaw_event_projections::{ProjectionCursor, ProjectionScope};
use ironclaw_host_api::{ThreadId, Timestamp};
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};
use serde::{Deserialize, Serialize};

use crate::{OutboundDeliveryId, ProjectionSubscriptionId, ProjectionUpdateRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboundPushKind {
    FinalReply,
    Progress,
    GateRequired,
    DeliveryStatus,
}

impl OutboundPushKind {
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::FinalReply => "final_reply",
            Self::Progress => "progress",
            Self::GateRequired => "gate_required",
            Self::DeliveryStatus => "delivery_status",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadNotificationTarget {
    pub target: ReplyTargetBindingRef,
    pub final_replies: bool,
    pub progress: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadNotificationPolicy {
    pub scope: TurnScope,
    pub targets: Vec<ThreadNotificationTarget>,
}

impl ThreadNotificationPolicy {
    pub fn default_for_scope(scope: TurnScope) -> Self {
        Self {
            scope,
            targets: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundPushTargetRequest {
    pub scope: TurnScope,
    pub turn_run_id: Option<TurnRunId>,
    pub reply_target: ReplyTargetBindingRef,
    pub kind: OutboundPushKind,
    pub projection_ref: ProjectionUpdateRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundPushCandidate {
    pub thread_id: ThreadId,
    pub turn_run_id: Option<TurnRunId>,
    pub target: ReplyTargetBindingRef,
    pub kind: OutboundPushKind,
    pub projection_ref: ProjectionUpdateRef,
    pub requires_reply_target_revalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundPushPlan {
    pub candidates: Vec<OutboundPushCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionSubscriptionRecord {
    pub subscription_id: ProjectionSubscriptionId,
    pub actor: TurnActor,
    pub scope: ProjectionScope,
    pub thread_id: ThreadId,
    pub cursor: Option<ProjectionCursor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadSubscriptionCursorRequest {
    pub subscription_id: ProjectionSubscriptionId,
    pub actor: TurnActor,
    pub scope: ProjectionScope,
    pub thread_id: ThreadId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvanceSubscriptionCursorRequest {
    pub subscription_id: ProjectionSubscriptionId,
    pub actor: TurnActor,
    pub thread_id: ThreadId,
    pub cursor: ProjectionCursor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboundDeliveryStatus {
    Pending,
    Delivered,
    Failed,
    DeadLettered,
}

impl OutboundDeliveryStatus {
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Delivered => "delivered",
            Self::Failed => "failed",
            Self::DeadLettered => "dead_lettered",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryFailureKind {
    AuthorizationRevoked,
    TransportUnavailable,
    RateLimited,
    Rejected,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundDeliveryAttempt {
    pub delivery_id: OutboundDeliveryId,
    pub scope: TurnScope,
    pub candidate: OutboundPushCandidate,
    pub status: OutboundDeliveryStatus,
    pub attempted_at: Timestamp,
    pub failure_kind: Option<DeliveryFailureKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateDeliveryStatusRequest {
    pub delivery_id: OutboundDeliveryId,
    pub scope: TurnScope,
    pub status: OutboundDeliveryStatus,
    pub updated_at: Timestamp,
    pub failure_kind: Option<DeliveryFailureKind>,
}
