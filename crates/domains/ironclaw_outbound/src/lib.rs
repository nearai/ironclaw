//! Outbound egress and projection subscription policy storage.
//!
//! This crate stores metadata-only Reborn outbound state: per-thread
//! notification policy, projection subscription cursors, and delivery attempt
//! status. It never owns transport delivery, transcript content, projection
//! payloads, prompts, tool I/O, secrets, host paths, or backend detail strings.

mod communication_preferences;
mod delivered_gate_routes;
mod delivery_resolution;
mod delivery_targets;
mod error;
mod ids;
mod model_channel_delivery;
mod outbound_state_store;
mod reply_attachment_intents;
mod resolution_engine;
mod run_delivery_cleanup;
mod service;
mod store;
mod triggered_run_delivery;
mod types;
mod validation;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use communication_preferences::{
    CommunicationPreferenceKey, CommunicationPreferenceRecord, CommunicationPreferenceRepository,
    CommunicationPreferenceVersion, DeliveryDefaultScope, NOTIFICATION_TARGETS_CAP,
    VersionedCommunicationPreferenceRecord, WriteCommunicationPreferenceRequest,
};
pub use delivered_gate_routes::{
    DELIVERED_GATE_ROUTE_TTL, DeliveredGateRouteRecord, DeliveredGateRouteStore,
    NoopDeliveredGateRouteStore,
};
pub use delivery_resolution::{
    CommunicationDeliveryCandidate, CommunicationDeliveryIntent, CommunicationDeliveryResolution,
    CommunicationDeliveryResolutionRequest, CommunicationModality, DeliveryTargetCapabilities,
    RequestedOutboundContext, RequestedOutboundKind, RunNotificationContext,
    RunNotificationEventKind, RunNotificationOrigin, SourceRouteContext, SystemEventReasonCode,
};
pub use delivery_targets::{
    MutableOutboundDeliveryTargetRegistry, OutboundDeliveryTargetChannel,
    OutboundDeliveryTargetDescription, OutboundDeliveryTargetDisplayName,
    OutboundDeliveryTargetEntry, OutboundDeliveryTargetId, OutboundDeliveryTargetOwner,
    OutboundDeliveryTargetProvider, OutboundDeliveryTargetRegistrationOutcome,
    OutboundDeliveryTargetRegistry, OutboundDeliveryTargetScope, OutboundDeliveryTargetSummary,
};
pub use error::OutboundError;
pub use ids::{OutboundDeliveryId, ProjectionSubscriptionId, ProjectionUpdateRef};
pub use model_channel_delivery::{
    MODEL_DELIVERY_MAX_CONTENT_BYTES, MODEL_DELIVERY_PER_RUN_CAP, ModelChannelDelivery,
    ModelChannelDeliveryError, ModelChannelDeliveryEvidence, ModelChannelDeliveryRequest,
};
pub use outbound_state_store::OutboundStateStore;
pub use reply_attachment_intents::{
    ReplyAttachmentHandle, ReplyAttachmentIntent, ReplyAttachmentIntentPort,
};
pub use run_delivery_cleanup::{
    MAX_RUN_DELIVERY_CLEANUP_RECORDS, RunDeliveryCleanupRecord, RunDeliveryCleanupRequest,
};
pub use service::{
    OutboundPolicyService, ReplyTargetBindingValidator, ThreadProjectionAccessPolicy,
};
pub use store::OutboundStateStorePort;
pub use triggered_run_delivery::{
    TriggeredFireFailureDeliveryRequest, TriggeredRunDelivery, TriggeredRunDeliveryOutcomeKind,
    TriggeredRunDeliveryRecord, TriggeredRunDeliveryRequest, TriggeredRunDeliveryStore,
};
pub use types::{
    AdvanceSubscriptionCursorRequest, ClaimDeliveryAttemptForSendRequest, DeliveryFailureKind,
    LoadSubscriptionCursorRequest, OutboundDeliveryAttempt, OutboundDeliveryDecision,
    OutboundDeliveryStatus, OutboundPushCandidate, OutboundPushKind, OutboundPushPlan,
    OutboundPushTargetRequest, PrepareCommunicationDeliveryRequest, PrepareOutboundDeliveryRequest,
    ProjectionSubscriptionRecord, ProjectionSubscriptionRequest, RecoverInterruptedDeliveryRequest,
    ReplyTargetBindingClaim, ReplyTargetValidationRequest, ThreadNotificationPolicy,
    ThreadNotificationTarget, ThreadProjectionAccessClaim, ThreadProjectionAccessGrant,
    ThreadProjectionAccessRequest, UpdateDeliveryStatusRequest, ValidatedReplyTargetBinding,
};
