//! Outbound egress and projection subscription policy storage.
//!
//! This crate stores metadata-only Reborn outbound state: per-thread
//! notification policy, projection subscription cursors, and delivery attempt
//! status. It never owns transport delivery, transcript content, projection
//! payloads, prompts, tool I/O, secrets, host paths, or backend detail strings.

mod error;
mod ids;
mod memory;
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
pub use ids::{OutboundDeliveryId, ProjectionSubscriptionId, ProjectionUpdateRef};
pub use memory::InMemoryOutboundStateStore;
pub use store::OutboundStateStore;
pub use types::{
    AdvanceSubscriptionCursorRequest, DeliveryFailureKind, LoadSubscriptionCursorRequest,
    OutboundDeliveryAttempt, OutboundDeliveryStatus, OutboundPushCandidate, OutboundPushKind,
    OutboundPushPlan, OutboundPushTargetRequest, ProjectionSubscriptionRecord,
    ThreadNotificationPolicy, ThreadNotificationTarget, UpdateDeliveryStatusRequest,
};

#[cfg(feature = "libsql")]
pub use libsql_store::LibSqlOutboundStateStore;
#[cfg(feature = "postgres")]
pub use postgres_store::PostgresOutboundStateStore;
