use super::*;

#[derive(Default)]
pub(crate) struct DenyingAccessPolicy {
    calls: Mutex<usize>,
}

impl DenyingAccessPolicy {
    pub(crate) fn calls(&self) -> usize {
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
pub(crate) struct CountingUpdateSource {
    calls: Mutex<usize>,
}

impl CountingUpdateSource {
    pub(crate) fn calls(&self) -> usize {
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

pub(crate) struct PointerUpdateSource {
    sender: tokio::sync::broadcast::Sender<Arc<ProductProjectionEnvelope>>,
}

impl PointerUpdateSource {
    pub(crate) fn new(capacity: usize) -> Self {
        let (sender, _) = tokio::sync::broadcast::channel(capacity.max(1));
        Self { sender }
    }

    pub(crate) fn publish_shared(
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

pub(crate) struct FailingUpdateSource;

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

pub(crate) struct FakeProjectionService {
    scope: ProjectionScope,
    calls: Mutex<Vec<&'static str>>,
}

impl FakeProjectionService {
    pub(crate) fn new(scope: ProjectionScope) -> Self {
        Self {
            scope,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn calls(&self) -> Vec<&'static str> {
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

pub(crate) struct ScopeMismatchProjectionService {
    scope: ProjectionScope,
}

impl ScopeMismatchProjectionService {
    pub(crate) fn new(scope: ProjectionScope) -> Self {
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

pub(crate) struct PayloadThreadMismatchProjectionService;

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

pub(crate) struct FailingUpdatesProjectionService {
    pub(crate) error: ProjectionError,
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
        Err(clone_projection_error(&self.error))
    }
}

pub(crate) struct FailingSnapshotProjectionService {
    pub(crate) error: ProjectionError,
}

#[async_trait]
impl EventProjectionService for FailingSnapshotProjectionService {
    async fn snapshot(
        &self,
        _request: ProjectionRequest,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        Err(clone_projection_error(&self.error))
    }

    async fn updates(
        &self,
        request: ProjectionRequest,
    ) -> Result<ProjectionReplay, ProjectionError> {
        Ok(replay(&request.scope, 2, 3))
    }
}

pub(crate) struct TruncatedProjectionService {
    scope: ProjectionScope,
    truncate_snapshot: bool,
    truncate_replay: bool,
}

impl TruncatedProjectionService {
    pub(crate) fn snapshot(scope: ProjectionScope) -> Self {
        Self {
            scope,
            truncate_snapshot: true,
            truncate_replay: false,
        }
    }

    pub(crate) fn replay(scope: ProjectionScope) -> Self {
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

pub(crate) struct SnapshotPublishingProjectionService {
    scope: ProjectionScope,
    source: Arc<InMemoryProjectionUpdateSource>,
}

impl SnapshotPublishingProjectionService {
    pub(crate) fn new(scope: ProjectionScope, source: Arc<InMemoryProjectionUpdateSource>) -> Self {
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

pub(crate) struct StaticSnapshotProjectionService {
    snapshot: ProjectionSnapshot,
}

impl StaticSnapshotProjectionService {
    pub(crate) fn new(snapshot: ProjectionSnapshot) -> Self {
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

pub(crate) struct ChangingSnapshotProjectionService {
    calls: Mutex<usize>,
}

impl ChangingSnapshotProjectionService {
    pub(crate) fn new() -> Self {
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

pub(crate) struct RejectLiveUpdateRedactionValidator;

impl ProjectionRedactionValidator for RejectLiveUpdateRedactionValidator {
    fn validate(&self, envelope: &ProductProjectionEnvelope) -> Result<(), ProjectionStreamError> {
        match envelope {
            ProductProjectionEnvelope::ThreadUpdates(_) => Err(ProjectionStreamError::Redaction),
            _ => Ok(()),
        }
    }
}

pub(crate) struct RejectSnapshotRedactionValidator;

impl ProjectionRedactionValidator for RejectSnapshotRedactionValidator {
    fn validate(&self, envelope: &ProductProjectionEnvelope) -> Result<(), ProjectionStreamError> {
        match envelope {
            ProductProjectionEnvelope::ThreadSnapshot(_) => Err(ProjectionStreamError::Redaction),
            _ => Ok(()),
        }
    }
}

pub(crate) struct SourceFailingLiveUpdateValidator;

impl ProjectionRedactionValidator for SourceFailingLiveUpdateValidator {
    fn validate(&self, envelope: &ProductProjectionEnvelope) -> Result<(), ProjectionStreamError> {
        match envelope {
            ProductProjectionEnvelope::ThreadUpdates(_) => Err(ProjectionStreamError::Source),
            _ => Ok(()),
        }
    }
}

pub(crate) struct RejectTruncatedSnapshotValidator;

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
pub(crate) struct CountingRedactionValidator {
    calls: Mutex<usize>,
}

impl CountingRedactionValidator {
    pub(crate) fn calls(&self) -> usize {
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
pub(crate) enum FailingOutboundKind {
    AccessDenied,
    InvalidRequest,
    Backend,
}

pub(crate) struct FailingOutboundStore {
    pub(crate) kind: FailingOutboundKind,
}

impl FailingOutboundStore {
    fn error(&self) -> OutboundError {
        match self.kind {
            FailingOutboundKind::AccessDenied => OutboundError::AccessDenied,
            FailingOutboundKind::InvalidRequest => OutboundError::InvalidRequest {
                reason: "bad request",
            },
            FailingOutboundKind::Backend => OutboundError::Backend,
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
