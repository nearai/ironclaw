use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_event_projections::{
    EventProjectionService, ProjectionCursor, ProjectionError, ProjectionReplay, ProjectionRequest,
    ProjectionScope, ProjectionSnapshot, RunProjectionStatus, RunStatusProjection, ThreadTimeline,
    TimelineEntry, TimelineEntryKind,
};
use ironclaw_event_streams::{
    AllowAllProjectionAccessPolicy, EventStreamManager, InMemoryProjectionStreamAdmissionPolicy,
    InMemoryProjectionUpdateSource, LagReason, NoExposureProjectionRedactionValidator,
    ProductProjectionEnvelope, ProjectionAccessPolicy, ProjectionAccessRequest,
    ProjectionFetchRequest, ProjectionLiveUpdateRequest, ProjectionRedactionValidator,
    ProjectionStreamAdmissionPolicy, ProjectionStreamAdmissionRequest, ProjectionStreamError,
    ProjectionStreamItem, ProjectionStreamLimits, ProjectionSubscribeRequest, ProjectionTarget,
    ProjectionUpdateSource, ProjectionViewClass, PushCandidatesForUpdateRequest,
    SubscriberCapabilities, keep_alive_item,
};
use ironclaw_events::{EventCursor, EventStreamKey, ReadScope};
use ironclaw_host_api::{
    CapabilityId, ExtensionId, InvocationId, MissionId, ProjectId, RuntimeKind, TenantId, ThreadId,
    UserId,
};
use ironclaw_outbound::{
    AdvanceSubscriptionCursorRequest, InMemoryOutboundStateStore, LoadSubscriptionCursorRequest,
    OutboundDeliveryAttempt, OutboundError, OutboundPushKind, OutboundPushPlan,
    OutboundPushTargetRequest, OutboundStateStore, ProjectionSubscriptionRecord,
    ProjectionUpdateRef, ThreadNotificationPolicy, ThreadNotificationTarget,
    UpdateDeliveryStatusRequest,
};
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnScope};
use tokio::{
    sync::Barrier,
    time::{Duration, timeout},
};
