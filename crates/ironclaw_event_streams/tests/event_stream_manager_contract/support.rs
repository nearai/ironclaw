pub(crate) use std::sync::{Arc, Mutex};

pub(crate) use async_trait::async_trait;
pub(crate) use ironclaw_event_projections::{
    EventProjectionService, ProjectionCursor, ProjectionError, ProjectionReplay, ProjectionRequest,
    ProjectionScope, ProjectionSnapshot, RunProjectionStatus, RunStatusProjection, ThreadTimeline,
    TimelineEntry, TimelineEntryKind,
};
pub(crate) use ironclaw_event_streams::{
    AllowAllProjectionAccessPolicy, EventStreamManager, InMemoryProjectionStreamAdmissionPolicy,
    InMemoryProjectionUpdateSource, LagReason, NoExposureProjectionRedactionValidator,
    ProductProjectionEnvelope, ProjectionAccessPolicy, ProjectionAccessRequest,
    ProjectionFetchRequest, ProjectionLiveUpdateRequest, ProjectionRedactionValidator,
    ProjectionStreamAdmissionPolicy, ProjectionStreamAdmissionRequest, ProjectionStreamError,
    ProjectionStreamItem, ProjectionStreamLimits, ProjectionSubscribeRequest, ProjectionTarget,
    ProjectionUpdateSource, ProjectionViewClass, PushCandidatesForUpdateRequest,
    SubscriberCapabilities, keep_alive_item,
};
pub(crate) use ironclaw_events::{EventCursor, EventStreamKey, ReadScope};
pub(crate) use ironclaw_host_api::{
    CapabilityId, ExtensionId, InvocationId, MissionId, ProjectId, RuntimeKind, TenantId, ThreadId,
    UserId,
};
pub(crate) use ironclaw_outbound::{
    AdvanceSubscriptionCursorRequest, InMemoryOutboundStateStore, LoadSubscriptionCursorRequest,
    OutboundDeliveryAttempt, OutboundError, OutboundPushKind, OutboundPushPlan,
    OutboundPushTargetRequest, OutboundStateStore, ProjectionSubscriptionRecord,
    ProjectionUpdateRef, ThreadNotificationPolicy, ThreadNotificationTarget,
    UpdateDeliveryStatusRequest,
};
pub(crate) use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnScope};
pub(crate) use tokio::{
    sync::Barrier,
    time::{Duration, timeout},
};

#[path = "support/fakes.rs"]
mod fakes;
#[path = "support/helpers.rs"]
mod helpers;
#[path = "support/managers.rs"]
mod managers;

pub(crate) use fakes::*;
pub(crate) use helpers::*;
pub(crate) use managers::*;
