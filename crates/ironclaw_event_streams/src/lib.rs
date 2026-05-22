//! Transport-neutral Reborn projection streams.
//!
//! This crate composes product-safe projection DTOs with access, admission,
//! live-update, redaction, and outbound-candidate seams. It intentionally does
//! not render SSE/WebSocket/channel frames and does not read durable logs
//! directly.

mod admission;
mod error;
mod keys;
mod manager;
mod redaction;
mod types;
mod update_source;

pub use admission::{
    AllowAllProjectionAccessPolicy, InMemoryProjectionStreamAdmissionPolicy,
    ProjectionAccessPolicy, ProjectionAccessRequest, ProjectionStreamAdmissionPermit,
    ProjectionStreamAdmissionPolicy, ProjectionStreamAdmissionRequest, ProjectionStreamLimits,
};
pub use error::ProjectionStreamError;
pub use manager::EventStreamManager;
pub use redaction::{NoExposureProjectionRedactionValidator, ProjectionRedactionValidator};
pub use types::{
    DebugProjectionPayload, DeliveryProjectionStatus, DeliveryStatusProjectionPayload, LagReason,
    ProductProjectionEnvelope, ProjectionFetchRequest, ProjectionFetchResponse,
    ProjectionStreamItem, ProjectionSubscribeRequest, ProjectionSubscription, ProjectionTarget,
    ProjectionViewClass, PushCandidatesForUpdateRequest, SubscriberCapabilities, keep_alive_item,
};
pub use update_source::{
    InMemoryProjectionUpdateSource, ProjectionLiveUpdateRequest, ProjectionUpdateSource,
};
