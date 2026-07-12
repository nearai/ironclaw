//! Outbound egress and projection subscription policy storage.
//!
//! This crate stores metadata-only Reborn outbound state: per-thread
//! notification policy, projection subscription cursors, and delivery attempt
//! status. It never owns transport delivery, transcript content, projection
//! payloads, prompts, tool I/O, secrets, host paths, or backend detail strings.

mod communication_preferences;
mod delivered_gate_routes;
mod delivery_resolution;
mod error;
mod filesystem_store;
mod ids;
mod memory;
mod resolution_engine;
mod service;
mod store;
mod triggered_run_delivery;
mod types;
mod validation;

pub use communication_preferences::{
    CommunicationPreferenceKey, CommunicationPreferenceRecord, CommunicationPreferenceRepository,
    CommunicationPreferenceVersion, DeliveryDefaultScope, VersionedCommunicationPreferenceRecord,
    WriteCommunicationPreferenceRequest,
};
pub use delivered_gate_routes::{
    DELIVERED_GATE_ROUTE_TTL, DeliveredGateRouteRecord, DeliveredGateRouteStore,
    InMemoryDeliveredGateRouteStore,
};
pub use delivery_resolution::{
    CommunicationDeliveryCandidate, CommunicationDeliveryIntent, CommunicationDeliveryKind,
    CommunicationDeliveryResolution, CommunicationDeliveryResolutionRequest, CommunicationModality,
    DeliveryTargetCapabilities, RequestedOutboundContext, RequestedOutboundKind,
    RunNotificationContext, RunNotificationEventKind, RunNotificationOrigin, SourceRouteContext,
    SystemEventReasonCode, TriggerCommunicationContext, TriggerSourceKind,
};
pub use error::OutboundError;
pub use filesystem_store::FilesystemOutboundStateStore;
pub use ids::{
    OutboundDeliveryId, ProjectionSubscriptionId, ProjectionUpdateRef, TriggerFireSlot,
    TriggerOriginRef,
};
pub use memory::InMemoryOutboundStateStore;
pub use service::{
    OutboundPolicyService, ReplyTargetBindingValidator, ThreadProjectionAccessPolicy,
};
pub use store::OutboundStateStore;
pub use triggered_run_delivery::{
    InMemoryTriggeredRunDeliveryStore, TriggeredRunDeliveryOutcomeKind, TriggeredRunDeliveryRecord,
    TriggeredRunDeliveryStore,
};
pub use types::{
    AdvanceSubscriptionCursorRequest, DeliveryFailureKind, LoadSubscriptionCursorRequest,
    OutboundDeliveryAttempt, OutboundDeliveryDecision, OutboundDeliveryStatus,
    OutboundPushCandidate, OutboundPushKind, OutboundPushPlan, OutboundPushTargetRequest,
    PrepareCommunicationDeliveryRequest, PrepareOutboundDeliveryRequest,
    ProjectionSubscriptionRecord, ProjectionSubscriptionRequest, ReplyTargetBindingClaim,
    ReplyTargetValidationRequest, ThreadNotificationPolicy, ThreadNotificationTarget,
    ThreadProjectionAccessClaim, ThreadProjectionAccessGrant, ThreadProjectionAccessRequest,
    UpdateDeliveryStatusRequest, ValidatedReplyTargetBinding,
};
