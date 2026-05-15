//! Outbound egress and projection subscription policy storage.
//!
//! This crate stores metadata-only Reborn outbound state: per-thread
//! notification policy, projection subscription cursors, and delivery attempt
//! status. It never owns transport delivery, transcript content, projection
//! payloads, prompts, tool I/O, secrets, host paths, or backend detail strings.

mod error;
mod filesystem_store;
mod ids;
mod memory;
mod service;
mod store;
mod types;
mod validation;

#[cfg(any(feature = "libsql", feature = "postgres"))]
mod db;
#[cfg(feature = "libsql")]
mod libsql_store;
#[cfg(feature = "postgres")]
mod postgres_store;

pub use error::OutboundError;
pub use filesystem_store::FilesystemOutboundStateStore;
pub use ids::{OutboundDeliveryId, ProjectionSubscriptionId, ProjectionUpdateRef};
pub use memory::InMemoryOutboundStateStore;
pub use service::{
    OutboundPolicyService, ReplyTargetBindingValidator, ThreadProjectionAccessPolicy,
};
pub use store::OutboundStateStore;
pub use types::{
    AdvanceSubscriptionCursorRequest, DeliveryFailureKind, LoadSubscriptionCursorRequest,
    OutboundDeliveryAttempt, OutboundDeliveryDecision, OutboundDeliveryStatus,
    OutboundPushCandidate, OutboundPushKind, OutboundPushPlan, OutboundPushTargetRequest,
    PrepareOutboundDeliveryRequest, ProjectionSubscriptionRecord, ProjectionSubscriptionRequest,
    ReplyTargetBindingClaim, ReplyTargetValidationRequest, ThreadNotificationPolicy,
    ThreadNotificationTarget, ThreadProjectionAccessClaim, ThreadProjectionAccessGrant,
    ThreadProjectionAccessRequest, UpdateDeliveryStatusRequest, ValidatedReplyTargetBinding,
};

#[cfg(feature = "libsql")]
pub use libsql_store::LibSqlOutboundStateStore;
#[cfg(feature = "postgres")]
pub use postgres_store::PostgresOutboundStateStore;
