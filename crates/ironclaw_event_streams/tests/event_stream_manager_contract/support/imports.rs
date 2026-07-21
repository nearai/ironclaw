use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_event_projections::{
    CapabilityActivityProjection, CapabilityActivityStatus, EventProjectionService,
    ProjectionCursor, ProjectionError, ProjectionReplay, ProjectionRequest, ProjectionScope,
    ProjectionSnapshot, RunProjectionStatus, RunStatusProjection, ThreadTimeline, TimelineEntry,
    TimelineEntryKind,
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
use ironclaw_filesystem::{
    Fault, FaultInjecting, FilesystemOperation, InMemoryBackend, ScopedFilesystem,
};
use ironclaw_host_api::{
    CapabilityId, ExtensionId, InvocationId, MissionId, MountAlias, MountGrant, MountPermissions,
    MountView, ProjectId, RuntimeKind, TenantId, ThreadId, UserId, VirtualPath,
};
use ironclaw_outbound::test_support::in_memory_backed_outbound_state_store;
use ironclaw_outbound::{
    AdvanceSubscriptionCursorRequest, FilesystemOutboundStateStore, LoadSubscriptionCursorRequest,
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
