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
    ProjectionFetchRequest, ProjectionStreamError, ProjectionStreamItem, ProjectionStreamLimits,
    ProjectionSubscribeRequest, ProjectionTarget, ProjectionViewClass,
    PushCandidatesForUpdateRequest, SubscriberCapabilities, keep_alive_item,
};
use ironclaw_events::{EventCursor, EventStreamKey, ReadScope};
use ironclaw_host_api::{
    CapabilityId, ExtensionId, InvocationId, ProjectId, RuntimeKind, TenantId, ThreadId, UserId,
};
use ironclaw_outbound::{
    InMemoryOutboundStateStore, OutboundPushKind, OutboundStateStore, ProjectionUpdateRef,
    ThreadNotificationPolicy, ThreadNotificationTarget,
};
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnScope};
use tokio::time::{Duration, timeout};

#[tokio::test]
async fn authorized_subscription_without_cursor_emits_snapshot_then_ordered_updates() {
    let scope = projection_scope("thread-a");
    let manager = manager_with_source(
        scope.clone(),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
    );

    let mut subscription = manager
        .subscribe(subscribe_request(scope.clone(), None))
        .await
        .expect("authorized subscription");

    let first = subscription.next().await.expect("initial snapshot");
    assert!(matches!(first, ProjectionStreamItem::Snapshot(_)));

    manager
        .update_source
        .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
            &scope, 2, 3,
        )))
        .expect("publish live update");

    let second = subscription.next().await.expect("live update");
    match second {
        ProjectionStreamItem::Update(ProductProjectionEnvelope::ThreadUpdates(replay)) => {
            assert_eq!(replay.updates[0].cursor, EventCursor::new(2));
            assert_eq!(replay.next_cursor.runtime, EventCursor::new(3));
        }
        other => panic!("expected thread update, got {other:?}"),
    }
}

#[tokio::test]
async fn valid_cursor_emits_current_snapshot_plus_replayed_updates() {
    let scope = projection_scope("thread-a");
    let manager = manager(scope.clone());

    let mut subscription = manager
        .subscribe(subscribe_request(
            scope.clone(),
            Some(ProjectionCursor::for_scope(scope, EventCursor::new(1))),
        ))
        .await
        .expect("authorized resume");

    assert!(matches!(
        subscription.next().await,
        Some(ProjectionStreamItem::Snapshot(_))
    ));
    match subscription.next().await.expect("replayed update") {
        ProjectionStreamItem::Update(ProductProjectionEnvelope::ThreadUpdates(replay)) => {
            assert_eq!(replay.updates[0].kind, TimelineEntryKind::DispatchSucceeded);
        }
        other => panic!("expected replayed update, got {other:?}"),
    }
}

#[tokio::test]
async fn fetch_snapshot_returns_authorized_product_snapshot() {
    let scope = projection_scope("thread-a");
    let manager = manager(scope.clone());

    let response = manager
        .fetch_snapshot(fetch_request(scope))
        .await
        .expect("authorized fetch");

    match response.snapshot {
        ProductProjectionEnvelope::ThreadSnapshot(snapshot) => {
            assert_eq!(snapshot.next_cursor, response.cursor);
            assert_eq!(snapshot.timeline.entries.len(), 1);
        }
        other => panic!("expected thread snapshot, got {other:?}"),
    }
}

#[tokio::test]
async fn access_policy_runs_before_projection_or_live_subscription() {
    let scope = projection_scope("thread-a");
    let projection = Arc::new(FakeProjectionService::new(scope.clone()));
    let access = Arc::new(DenyingAccessPolicy::default());
    let update_source = Arc::new(CountingUpdateSource::default());
    let manager = EventStreamManager::new(
        Arc::clone(&projection),
        Arc::clone(&access),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::clone(&update_source),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let error = manager
        .subscribe(subscribe_request(
            scope.clone(),
            Some(ProjectionCursor::for_scope(scope, EventCursor::new(99))),
        ))
        .await
        .expect_err("access denial");

    assert!(matches!(error, ProjectionStreamError::AccessDenied));
    assert_eq!(access.calls(), 1);
    assert_eq!(projection.calls(), Vec::<&'static str>::new());
    assert_eq!(update_source.calls(), 0);
}

#[tokio::test]
async fn debug_admin_view_is_denied_without_widening_product_thread_streams() {
    let scope = projection_scope("thread-a");
    let manager = manager(scope.clone());
    let error = manager
        .subscribe(ProjectionSubscribeRequest {
            view: ProjectionViewClass::DebugSupport,
            ..subscribe_request(scope, None)
        })
        .await
        .expect_err("debug view denied");

    assert!(matches!(error, ProjectionStreamError::AccessDenied));
}

#[tokio::test]
async fn stale_cursor_after_valid_access_rebases_to_fresh_snapshot() {
    let scope = projection_scope("thread-a");
    let stale = ProjectionCursor::for_scope(scope.clone(), EventCursor::new(99));
    let manager = manager(scope);

    let mut subscription = manager
        .subscribe(subscribe_request(stale.scope.clone(), Some(stale.clone())))
        .await
        .expect("authorized subscription");

    match subscription.next().await.expect("rebase item") {
        ProjectionStreamItem::RebaseRequired {
            rebased_from,
            snapshot_cursor,
            ..
        } => {
            assert_eq!(rebased_from, Some(stale));
            assert_eq!(snapshot_cursor.runtime, EventCursor::new(10));
        }
        other => panic!("expected rebase item, got {other:?}"),
    }
}

#[tokio::test]
async fn foreign_cursor_rebases_without_echoing_foreign_cursor() {
    let requested_scope = projection_scope("thread-a");
    let foreign_scope = projection_scope("thread-b");
    let manager = manager(requested_scope.clone());

    let mut subscription = manager
        .subscribe(subscribe_request(
            requested_scope,
            Some(ProjectionCursor::for_scope(
                foreign_scope,
                EventCursor::new(7),
            )),
        ))
        .await
        .expect("authorized subscription");

    match subscription.next().await.expect("rebase item") {
        ProjectionStreamItem::RebaseRequired { rebased_from, .. } => {
            assert_eq!(rebased_from, None);
        }
        other => panic!("expected safe rebase item, got {other:?}"),
    }
}

#[tokio::test]
async fn broadcast_lag_emits_explicit_lagged_item() {
    let scope = projection_scope("thread-a");
    let source = Arc::new(InMemoryProjectionUpdateSource::new(1));
    let manager = manager_with_source(scope.clone(), Arc::clone(&source));
    let mut subscription = manager
        .subscribe(ProjectionSubscribeRequest {
            capabilities: SubscriberCapabilities { buffer_capacity: 1 },
            ..subscribe_request(scope.clone(), None)
        })
        .await
        .expect("authorized subscription");

    source
        .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
            &scope, 2, 2,
        )))
        .unwrap();
    source
        .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
            &scope, 3, 3,
        )))
        .unwrap();
    source
        .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
            &scope, 4, 4,
        )))
        .unwrap();

    assert!(matches!(
        subscription.next().await,
        Some(ProjectionStreamItem::Snapshot(_))
    ));
    let mut lagged = None;
    for _ in 0..3 {
        let item = timeout(Duration::from_secs(1), subscription.next())
            .await
            .expect("next stream item")
            .expect("next stream item");
        if matches!(item, ProjectionStreamItem::Lagged { .. }) {
            lagged = Some(item);
            break;
        }
    }

    match lagged.expect("lag marker") {
        ProjectionStreamItem::Lagged { reason, .. } => {
            assert_eq!(reason, LagReason::SourceLagged);
        }
        other => panic!("expected lag marker, got {other:?}"),
    }
}

#[tokio::test]
async fn stream_admission_denial_is_structured_and_product_safe() {
    let scope = projection_scope("thread-a");
    let projection = Arc::new(FakeProjectionService::new(scope.clone()));
    let manager = EventStreamManager::new(
        projection,
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::new(
            ProjectionStreamLimits {
                per_tenant: 1,
                per_actor: 1,
                per_scope: 1,
                global: 1,
            },
        )),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let _first = manager
        .subscribe(subscribe_request(scope.clone(), None))
        .await
        .expect("first subscription admitted");
    let error = manager
        .subscribe(subscribe_request(scope, None))
        .await
        .expect_err("second subscription rejected");

    assert!(matches!(error, ProjectionStreamError::AdmissionDenied));
}

#[tokio::test]
async fn redaction_gate_blocks_sentinel_payloads_at_stream_boundary() {
    let scope = projection_scope("thread-a");
    let source = Arc::new(InMemoryProjectionUpdateSource::new(8));
    let manager = manager_with_source(scope.clone(), Arc::clone(&source));
    let mut subscription = manager
        .subscribe(subscribe_request(scope.clone(), None))
        .await
        .expect("authorized subscription");
    assert!(matches!(
        subscription.next().await,
        Some(ProjectionStreamItem::Snapshot(_))
    ));

    source
        .publish(ProductProjectionEnvelope::Debug(
            ironclaw_event_streams::DebugProjectionPayload {
                cursor: ProjectionCursor::for_scope(scope, EventCursor::new(11)),
                redacted_summary: "SECRET_SENTINEL_sk_live".to_string(),
            },
        ))
        .expect("publish unsafe payload");

    match subscription.next().await.expect("redaction marker") {
        ProjectionStreamItem::Lagged { reason, .. } => {
            assert_eq!(reason, LagReason::RedactionBlocked);
        }
        other => panic!("expected redaction lag marker, got {other:?}"),
    }
}

#[tokio::test]
async fn push_candidates_are_separate_from_subscriptions_and_policy_gated() {
    let scope = projection_scope("thread-a");
    let turn_scope = turn_scope("thread-a");
    let outbound = Arc::new(InMemoryOutboundStateStore::default());
    let manager = EventStreamManager::new(
        Arc::new(FakeProjectionService::new(scope)),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::clone(&outbound),
    );

    let final_reply = manager
        .push_candidates_for_update(push_request(&turn_scope, OutboundPushKind::FinalReply))
        .await
        .expect("final reply candidates");
    assert_eq!(final_reply.len(), 1);
    assert_eq!(final_reply[0].target, reply_target("reply-default"));

    let progress = manager
        .push_candidates_for_update(push_request(&turn_scope, OutboundPushKind::Progress))
        .await
        .expect("progress candidates");
    assert!(progress.is_empty());

    outbound
        .put_thread_notification_policy(ThreadNotificationPolicy {
            scope: turn_scope.clone(),
            targets: vec![ThreadNotificationTarget {
                target: reply_target("reply-progress"),
                final_replies: false,
                progress: true,
            }],
        })
        .await
        .expect("store policy");

    let progress = manager
        .push_candidates_for_update(push_request(&turn_scope, OutboundPushKind::Progress))
        .await
        .expect("progress candidates");
    assert_eq!(progress.len(), 1);
    assert_eq!(progress[0].target, reply_target("reply-progress"));
}

#[test]
fn keepalive_items_do_not_advance_projection_cursor_or_carry_payload() {
    assert_eq!(keep_alive_item(), ProjectionStreamItem::KeepAlive);
}

struct TestManager {
    inner: EventStreamManager,
    update_source: Arc<InMemoryProjectionUpdateSource>,
}

impl std::ops::Deref for TestManager {
    type Target = EventStreamManager;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

fn manager(scope: ProjectionScope) -> TestManager {
    manager_with_source(scope, Arc::new(InMemoryProjectionUpdateSource::new(8)))
}

fn manager_with_source(
    scope: ProjectionScope,
    update_source: Arc<InMemoryProjectionUpdateSource>,
) -> TestManager {
    TestManager {
        inner: EventStreamManager::new(
            Arc::new(FakeProjectionService::new(scope)),
            Arc::new(AllowAllProjectionAccessPolicy),
            Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
            Arc::clone(&update_source),
            Arc::new(NoExposureProjectionRedactionValidator),
            Arc::new(InMemoryOutboundStateStore::default()),
        ),
        update_source,
    }
}

#[derive(Default)]
struct DenyingAccessPolicy {
    calls: Mutex<usize>,
}

impl DenyingAccessPolicy {
    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

#[async_trait]
impl ProjectionAccessPolicy for DenyingAccessPolicy {
    async fn authorize(
        &self,
        _request: ProjectionAccessRequest,
    ) -> Result<(), ProjectionStreamError> {
        *self.calls.lock().unwrap() += 1;
        Err(ProjectionStreamError::AccessDenied)
    }
}

#[derive(Default)]
struct CountingUpdateSource {
    calls: Mutex<usize>,
}

impl CountingUpdateSource {
    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

#[async_trait]
impl ironclaw_event_streams::ProjectionUpdateSource for CountingUpdateSource {
    async fn subscribe(
        &self,
        _request: ironclaw_event_streams::ProjectionLiveUpdateRequest,
    ) -> Result<tokio::sync::broadcast::Receiver<ProductProjectionEnvelope>, ProjectionStreamError>
    {
        *self.calls.lock().unwrap() += 1;
        Ok(InMemoryProjectionUpdateSource::new(1)
            .subscribe(_request)
            .await?)
    }
}

struct FakeProjectionService {
    scope: ProjectionScope,
    calls: Mutex<Vec<&'static str>>,
}

impl FakeProjectionService {
    fn new(scope: ProjectionScope) -> Self {
        Self {
            scope,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl EventProjectionService for FakeProjectionService {
    async fn snapshot(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        self.calls.lock().unwrap().push("snapshot");
        Ok(snapshot(&request.scope, 10))
    }

    async fn updates(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        self.calls.lock().unwrap().push("updates");
        let cursor = request.after.expect("test supplies cursor");
        if cursor.runtime == EventCursor::new(99) {
            return Err(ProjectionError::RebaseRequired {
                requested: Box::new(cursor),
                earliest: Box::new(ProjectionCursor::origin_for_scope(self.scope.clone())),
            });
        }
        Ok(replay(&request.scope, 2, 3))
    }
}

fn subscribe_request(
    scope: ProjectionScope,
    after_cursor: Option<ProjectionCursor>,
) -> ProjectionSubscribeRequest {
    ProjectionSubscribeRequest {
        actor: actor("user-a"),
        target: ProjectionTarget::Thread {
            thread_id: scope.read_scope.thread_id.clone().unwrap(),
        },
        scope,
        view: ProjectionViewClass::ProductThread,
        after_cursor,
        limit: 16,
        capabilities: SubscriberCapabilities { buffer_capacity: 2 },
    }
}

fn fetch_request(scope: ProjectionScope) -> ProjectionFetchRequest {
    ProjectionFetchRequest {
        actor: actor("user-a"),
        target: ProjectionTarget::Thread {
            thread_id: scope.read_scope.thread_id.clone().unwrap(),
        },
        scope,
        view: ProjectionViewClass::ProductThread,
        limit: 16,
    }
}

fn push_request(scope: &TurnScope, kind: OutboundPushKind) -> PushCandidatesForUpdateRequest {
    PushCandidatesForUpdateRequest {
        scope: scope.clone(),
        turn_run_id: None,
        reply_target: reply_target("reply-default"),
        kind,
        projection_ref: ProjectionUpdateRef::new("projection:update:1").unwrap(),
    }
}

fn snapshot(scope: &ProjectionScope, cursor: u64) -> ProjectionSnapshot {
    ProjectionSnapshot {
        timeline: ThreadTimeline {
            entries: vec![timeline_entry(
                scope,
                cursor,
                TimelineEntryKind::DispatchRequested,
            )],
        },
        runs: vec![run_status(scope, cursor)],
        next_cursor: ProjectionCursor::for_scope(scope.clone(), EventCursor::new(cursor)),
        truncated: false,
    }
}

fn replay(scope: &ProjectionScope, cursor: u64, next: u64) -> ProjectionReplay {
    ProjectionReplay {
        updates: vec![timeline_entry(
            scope,
            cursor,
            TimelineEntryKind::DispatchSucceeded,
        )],
        runs: vec![run_status(scope, next)],
        next_cursor: ProjectionCursor::for_scope(scope.clone(), EventCursor::new(next)),
        truncated: false,
    }
}

fn timeline_entry(scope: &ProjectionScope, cursor: u64, kind: TimelineEntryKind) -> TimelineEntry {
    TimelineEntry {
        cursor: EventCursor::new(cursor),
        event_id: ironclaw_events::RuntimeEventId::new(),
        timestamp: chrono::Utc::now(),
        kind,
        invocation_id: InvocationId::new(),
        thread_id: scope.read_scope.thread_id.clone(),
        capability_id: CapabilityId::new("script.echo").unwrap(),
        provider: Some(ExtensionId::new("script").unwrap()),
        runtime: Some(RuntimeKind::Script),
        process_id: None,
        output_bytes: Some(12),
        error_kind: None,
    }
}

fn run_status(scope: &ProjectionScope, cursor: u64) -> RunStatusProjection {
    RunStatusProjection {
        invocation_id: InvocationId::new(),
        capability_id: CapabilityId::new("script.echo").unwrap(),
        thread_id: scope.read_scope.thread_id.clone(),
        status: RunProjectionStatus::Completed,
        provider: Some(ExtensionId::new("script").unwrap()),
        runtime: Some(RuntimeKind::Script),
        process_id: None,
        error_kind: None,
        last_cursor: EventCursor::new(cursor),
        updated_at: chrono::Utc::now(),
    }
}

fn projection_scope(thread: &str) -> ProjectionScope {
    let thread_id = ThreadId::new(thread).unwrap();
    ProjectionScope {
        stream: EventStreamKey::new(
            TenantId::new("tenant-a").unwrap(),
            UserId::new("user-a").unwrap(),
            None,
        ),
        read_scope: ReadScope {
            project_id: Some(ProjectId::new("project-a").unwrap()),
            mission_id: None,
            thread_id: Some(thread_id),
            process_id: None,
        },
    }
}

fn turn_scope(thread: &str) -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant-a").unwrap(),
        None,
        Some(ProjectId::new("project-a").unwrap()),
        ThreadId::new(thread).unwrap(),
    )
}

fn actor(user: &str) -> TurnActor {
    TurnActor::new(UserId::new(user).unwrap())
}

fn reply_target(value: &str) -> ReplyTargetBindingRef {
    ReplyTargetBindingRef::new(value).unwrap()
}
