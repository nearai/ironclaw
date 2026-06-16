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

async fn assert_second_subscription_denied_by_admission(
    limits: ProjectionStreamLimits,
    first: ProjectionScope,
    second: ProjectionScope,
) {
    let manager = EventStreamManager::new(
        Arc::new(FakeProjectionService::new(first.clone())),
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::new(limits)),
        Arc::new(InMemoryProjectionUpdateSource::new(8)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    );

    let _first = manager
        .subscribe(subscribe_request_for_stream_user(first, None))
        .await
        .expect("first subscription admitted");
    let error = manager
        .subscribe(subscribe_request_for_stream_user(second, None))
        .await
        .expect_err("second subscription rejected by targeted admission limit");

    assert!(matches!(error, ProjectionStreamError::AdmissionDenied));
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
impl ProjectionUpdateSource for CountingUpdateSource {
    async fn subscribe(
        &self,
        request: ProjectionLiveUpdateRequest,
    ) -> Result<
        tokio::sync::broadcast::Receiver<Arc<ProductProjectionEnvelope>>,
        ProjectionStreamError,
    > {
        *self.calls.lock().unwrap() += 1;
        Ok(InMemoryProjectionUpdateSource::new(1)
            .subscribe(request)
            .await?)
    }
}

struct PointerUpdateSource {
    sender: tokio::sync::broadcast::Sender<Arc<ProductProjectionEnvelope>>,
}

impl PointerUpdateSource {
    fn new(capacity: usize) -> Self {
        let (sender, _) = tokio::sync::broadcast::channel(capacity.max(1));
        Self { sender }
    }

    fn publish_shared(
        &self,
        envelope: Arc<ProductProjectionEnvelope>,
    ) -> Result<usize, ProjectionStreamError> {
        self.sender
            .send(envelope)
            .map_err(|_| ProjectionStreamError::Source)
    }
}

#[async_trait]
impl ProjectionUpdateSource for PointerUpdateSource {
    async fn subscribe(
        &self,
        _request: ProjectionLiveUpdateRequest,
    ) -> Result<
        tokio::sync::broadcast::Receiver<Arc<ProductProjectionEnvelope>>,
        ProjectionStreamError,
    > {
        Ok(self.sender.subscribe())
    }
}

struct FailingUpdateSource;

#[async_trait]
impl ProjectionUpdateSource for FailingUpdateSource {
    async fn subscribe(
        &self,
        _request: ProjectionLiveUpdateRequest,
    ) -> Result<
        tokio::sync::broadcast::Receiver<Arc<ProductProjectionEnvelope>>,
        ProjectionStreamError,
    > {
        Err(ProjectionStreamError::Source)
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

struct InitialSnapshotRebaseProjectionService {
    earliest_scope: ProjectionScope,
    calls: Mutex<Vec<Option<EventCursor>>>,
}

impl InitialSnapshotRebaseProjectionService {
    fn new(scope: ProjectionScope) -> Self {
        Self {
            earliest_scope: scope.clone(),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn with_earliest_scope(earliest_scope: ProjectionScope) -> Self {
        Self {
            earliest_scope,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<Option<EventCursor>> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl EventProjectionService for InitialSnapshotRebaseProjectionService {
    async fn snapshot(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        self.calls
            .lock()
            .unwrap()
            .push(request.after.as_ref().map(|cursor| cursor.runtime));
        match request.after {
            None => Err(ProjectionError::RebaseRequired {
                requested: Box::new(ProjectionCursor::origin_for_scope(request.scope)),
                earliest: Box::new(ProjectionCursor::for_scope(
                    self.earliest_scope.clone(),
                    EventCursor::new(5),
                )),
            }),
            Some(cursor) if cursor.runtime == EventCursor::new(4) => {
                Ok(snapshot(&request.scope, 10))
            }
            Some(cursor) => Err(ProjectionError::RebaseRequired {
                requested: Box::new(cursor),
                earliest: Box::new(ProjectionCursor::for_scope(
                    self.earliest_scope.clone(),
                    EventCursor::new(5),
                )),
            }),
        }
    }

    async fn updates(
        &self,
        _request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        panic!("initial snapshot rebase test does not resume updates")
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

struct ScopeMismatchProjectionService {
    scope: ProjectionScope,
}

impl ScopeMismatchProjectionService {
    fn new(scope: ProjectionScope) -> Self {
        Self { scope }
    }
}

#[async_trait]
impl EventProjectionService for ScopeMismatchProjectionService {
    async fn snapshot(
        &self,
        _request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        Ok(snapshot(&self.scope, 10))
    }

    async fn updates(
        &self,
        _request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        Ok(replay(&self.scope, 2, 3))
    }
}

struct PayloadThreadMismatchProjectionService;

#[async_trait]
impl EventProjectionService for PayloadThreadMismatchProjectionService {
    async fn snapshot(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        Ok(snapshot_for_thread(&request.scope, 10, "thread-b"))
    }

    async fn updates(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        Ok(replay_for_thread(&request.scope, 2, 3, "thread-b"))
    }
}

struct ActivityThreadMismatchProjectionService;

#[async_trait]
impl EventProjectionService for ActivityThreadMismatchProjectionService {
    async fn snapshot(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        Ok(snapshot_with_activity_thread(&request.scope, 10, "thread-b"))
    }

    async fn updates(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        Ok(replay_with_activity_thread(&request.scope, 2, 3, "thread-b"))
    }
}

struct ActivityTransitionThreadMismatchProjectionService;

#[async_trait]
impl EventProjectionService for ActivityTransitionThreadMismatchProjectionService {
    async fn snapshot(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        Ok(snapshot(&request.scope, 10))
    }

    async fn updates(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        Ok(replay_with_activity_transition_thread(
            &request.scope,
            2,
            3,
            "thread-b",
        ))
    }
}

struct FailingUpdatesProjectionService {
    error: ProjectionError,
}

#[async_trait]
impl EventProjectionService for FailingUpdatesProjectionService {
    async fn snapshot(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        Ok(snapshot(&request.scope, 10))
    }

    async fn updates(
        &self,
        _request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        let error = match &self.error {
            ProjectionError::InvalidRequest { reason } => {
                ProjectionError::InvalidRequest { reason }
            }
            ProjectionError::MissingProjectionMetadata { field } => {
                ProjectionError::MissingProjectionMetadata { field: *field }
            }
            ProjectionError::RebaseRequired {
                requested,
                earliest,
            } => ProjectionError::RebaseRequired {
                requested: requested.clone(),
                earliest: earliest.clone(),
            },
            ProjectionError::TurnEventRebaseRequired {
                requested,
                earliest,
            } => ProjectionError::TurnEventRebaseRequired {
                requested: *requested,
                earliest: *earliest,
            },
            ProjectionError::Source { operation } => ProjectionError::Source { operation },
        };
        Err(error)
    }
}

struct FailingSnapshotProjectionService {
    error: ProjectionError,
}

#[async_trait]
impl EventProjectionService for FailingSnapshotProjectionService {
    async fn snapshot(
        &self,
        _request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        Err(match &self.error {
            ProjectionError::InvalidRequest { reason } => {
                ProjectionError::InvalidRequest { reason }
            }
            ProjectionError::MissingProjectionMetadata { field } => {
                ProjectionError::MissingProjectionMetadata { field: *field }
            }
            ProjectionError::RebaseRequired {
                requested,
                earliest,
            } => ProjectionError::RebaseRequired {
                requested: requested.clone(),
                earliest: earliest.clone(),
            },
            ProjectionError::TurnEventRebaseRequired {
                requested,
                earliest,
            } => ProjectionError::TurnEventRebaseRequired {
                requested: *requested,
                earliest: *earliest,
            },
            ProjectionError::Source { operation } => ProjectionError::Source { operation },
        })
    }

    async fn updates(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        Ok(replay(&request.scope, 2, 3))
    }
}

struct TruncatedProjectionService {
    scope: ProjectionScope,
    truncate_snapshot: bool,
    truncate_replay: bool,
}

impl TruncatedProjectionService {
    fn snapshot(scope: ProjectionScope) -> Self {
        Self {
            scope,
            truncate_snapshot: true,
            truncate_replay: false,
        }
    }

    fn replay(scope: ProjectionScope) -> Self {
        Self {
            scope,
            truncate_snapshot: false,
            truncate_replay: true,
        }
    }
}

#[async_trait]
impl EventProjectionService for TruncatedProjectionService {
    async fn snapshot(
        &self,
        _request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        let mut snapshot = snapshot(&self.scope, 10);
        snapshot.truncated = self.truncate_snapshot;
        Ok(snapshot)
    }

    async fn updates(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        let mut replay = replay(&request.scope, 2, 3);
        replay.truncated = self.truncate_replay;
        Ok(replay)
    }
}

struct SnapshotPublishingProjectionService {
    scope: ProjectionScope,
    source: Arc<InMemoryProjectionUpdateSource>,
}

impl SnapshotPublishingProjectionService {
    fn new(scope: ProjectionScope, source: Arc<InMemoryProjectionUpdateSource>) -> Self {
        Self { scope, source }
    }
}

#[async_trait]
impl EventProjectionService for SnapshotPublishingProjectionService {
    async fn snapshot(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        self.source
            .publish(ProductProjectionEnvelope::ThreadUpdates(replay(
                &self.scope,
                11,
                11,
            )))
            .expect("publish race update");
        Ok(snapshot(&request.scope, 10))
    }

    async fn updates(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        Ok(replay(&request.scope, 2, 3))
    }
}

struct StaticSnapshotProjectionService {
    snapshot: ProjectionSnapshot,
}

impl StaticSnapshotProjectionService {
    fn new(snapshot: ProjectionSnapshot) -> Self {
        Self { snapshot }
    }
}

#[async_trait]
impl EventProjectionService for StaticSnapshotProjectionService {
    async fn snapshot(
        &self,
        _request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        Ok(self.snapshot.clone())
    }

    async fn updates(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        Ok(replay(&request.scope, 2, 3))
    }
}

struct ChangingSnapshotProjectionService {
    calls: Mutex<usize>,
}

impl ChangingSnapshotProjectionService {
    fn new() -> Self {
        Self {
            calls: Mutex::new(0),
        }
    }
}

#[async_trait]
impl EventProjectionService for ChangingSnapshotProjectionService {
    async fn snapshot(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        let mut calls = self.calls.lock().unwrap();
        let mut snapshot = snapshot(&request.scope, 10);
        if *calls > 0 {
            snapshot.truncated = true;
        }
        *calls += 1;
        Ok(snapshot)
    }

    async fn updates(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        Ok(replay(&request.scope, 2, 3))
    }
}

struct RejectLiveUpdateRedactionValidator;

impl ProjectionRedactionValidator for RejectLiveUpdateRedactionValidator {
    fn validate(&self, envelope: &ProductProjectionEnvelope) -> Result<(), ProjectionStreamError> {
        match envelope {
            ProductProjectionEnvelope::ThreadUpdates(_) => Err(ProjectionStreamError::Redaction),
            _ => Ok(()),
        }
    }
}

struct RejectSnapshotRedactionValidator;

impl ProjectionRedactionValidator for RejectSnapshotRedactionValidator {
    fn validate(&self, envelope: &ProductProjectionEnvelope) -> Result<(), ProjectionStreamError> {
        match envelope {
            ProductProjectionEnvelope::ThreadSnapshot(_) => Err(ProjectionStreamError::Redaction),
            _ => Ok(()),
        }
    }
}

struct SourceFailingLiveUpdateValidator;

impl ProjectionRedactionValidator for SourceFailingLiveUpdateValidator {
    fn validate(&self, envelope: &ProductProjectionEnvelope) -> Result<(), ProjectionStreamError> {
        match envelope {
            ProductProjectionEnvelope::ThreadUpdates(_) => Err(ProjectionStreamError::Source),
            _ => Ok(()),
        }
    }
}

struct RejectTruncatedSnapshotValidator;

impl ProjectionRedactionValidator for RejectTruncatedSnapshotValidator {
    fn validate(&self, envelope: &ProductProjectionEnvelope) -> Result<(), ProjectionStreamError> {
        match envelope {
            ProductProjectionEnvelope::ThreadSnapshot(snapshot) if snapshot.truncated => {
                Err(ProjectionStreamError::Redaction)
            }
            _ => Ok(()),
        }
    }
}

#[derive(Default)]
struct CountingRedactionValidator {
    calls: Mutex<usize>,
}

impl CountingRedactionValidator {
    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

impl ProjectionRedactionValidator for CountingRedactionValidator {
    fn validate(&self, _envelope: &ProductProjectionEnvelope) -> Result<(), ProjectionStreamError> {
        *self.calls.lock().unwrap() += 1;
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum FailingOutboundKind {
    AccessDenied,
    InvalidRequest,
    Backend,
    PreferenceTargetMissing,
}

struct FailingOutboundStore {
    kind: FailingOutboundKind,
}

impl FailingOutboundStore {
    fn error(&self) -> OutboundError {
        match self.kind {
            FailingOutboundKind::AccessDenied => OutboundError::AccessDenied,
            FailingOutboundKind::InvalidRequest => OutboundError::InvalidRequest {
                reason: "bad request",
            },
            FailingOutboundKind::Backend => OutboundError::Backend,
            FailingOutboundKind::PreferenceTargetMissing => {
                OutboundError::PreferenceTargetMissing { kind: "approval" }
            }
        }
    }
}

#[async_trait]
impl OutboundStateStore for FailingOutboundStore {
    async fn put_thread_notification_policy(
        &self,
        _policy: ThreadNotificationPolicy,
    ) -> Result<(), OutboundError> {
        Err(self.error())
    }

    async fn load_thread_notification_policy(
        &self,
        _scope: TurnScope,
    ) -> Result<ThreadNotificationPolicy, OutboundError> {
        Err(self.error())
    }

    async fn plan_push_targets(
        &self,
        _request: OutboundPushTargetRequest,
    ) -> Result<OutboundPushPlan, OutboundError> {
        Err(self.error())
    }

    async fn upsert_subscription(
        &self,
        _record: ProjectionSubscriptionRecord,
    ) -> Result<(), OutboundError> {
        Err(self.error())
    }

    async fn load_subscription_cursor(
        &self,
        _request: LoadSubscriptionCursorRequest,
    ) -> Result<Option<ProjectionCursor>, OutboundError> {
        Err(self.error())
    }

    async fn advance_subscription_cursor(
        &self,
        _request: AdvanceSubscriptionCursorRequest,
    ) -> Result<(), OutboundError> {
        Err(self.error())
    }

    async fn record_delivery_attempt(
        &self,
        _attempt: OutboundDeliveryAttempt,
    ) -> Result<(), OutboundError> {
        Err(self.error())
    }

    async fn update_delivery_status(
        &self,
        _request: UpdateDeliveryStatusRequest,
    ) -> Result<(), OutboundError> {
        Err(self.error())
    }

    async fn list_delivery_attempts(
        &self,
        _scope: TurnScope,
    ) -> Result<Vec<OutboundDeliveryAttempt>, OutboundError> {
        Err(self.error())
    }
}
