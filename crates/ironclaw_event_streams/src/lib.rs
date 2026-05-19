//! Transport-neutral Reborn projection streams.
//!
//! This crate composes product-safe projection DTOs with access, admission,
//! live-update, redaction, and outbound-candidate seams. It intentionally does
//! not render SSE/WebSocket/channel frames and does not read durable logs
//! directly.

use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use ironclaw_event_projections::{
    EventProjectionService, ProjectionCursor, ProjectionError, ProjectionReplay, ProjectionRequest,
    ProjectionScope, ProjectionSnapshot,
};
use ironclaw_host_api::{InvocationId, MissionId, ProcessId, TenantId, ThreadId};
use ironclaw_outbound::{
    OutboundError, OutboundPushCandidate, OutboundPushKind, OutboundPushTargetRequest,
    OutboundStateStore, ProjectionUpdateRef,
};
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};
use serde::{Deserialize, Serialize};
use tokio::{
    sync::{broadcast, mpsc},
    time::sleep,
};

mod error;

pub use error::ProjectionStreamError;

const DEFAULT_SUBSCRIPTION_BUFFER: usize = 16;
const MIN_SUBSCRIPTION_BUFFER: usize = 1;
const MAX_SUBSCRIPTION_BUFFER: usize = 128;
const MAX_VALIDATION_CACHE_ENTRIES: usize = 1024;
const TERMINAL_LAG_SEND_TIMEOUT_MILLIS: u64 = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionFetchRequest {
    pub actor: TurnActor,
    pub scope: ProjectionScope,
    pub view: ProjectionViewClass,
    pub target: ProjectionTarget,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionFetchResponse {
    pub snapshot: ProductProjectionEnvelope,
    pub cursor: ProjectionCursor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionSubscribeRequest {
    pub actor: TurnActor,
    pub scope: ProjectionScope,
    pub view: ProjectionViewClass,
    pub target: ProjectionTarget,
    pub after_cursor: Option<ProjectionCursor>,
    pub limit: usize,
    pub capabilities: SubscriberCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriberCapabilities {
    pub buffer_capacity: usize,
}

impl Default for SubscriberCapabilities {
    fn default() -> Self {
        Self {
            buffer_capacity: DEFAULT_SUBSCRIPTION_BUFFER,
        }
    }
}

impl SubscriberCapabilities {
    fn bounded_buffer_capacity(&self) -> Result<usize, ProjectionStreamError> {
        if self.buffer_capacity > MAX_SUBSCRIPTION_BUFFER {
            return Err(ProjectionStreamError::InvalidRequest {
                reason: "projection subscription buffer capacity exceeds host maximum",
            });
        }
        Ok(self.buffer_capacity.max(MIN_SUBSCRIPTION_BUFFER))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushCandidatesForUpdateRequest {
    pub scope: TurnScope,
    pub turn_run_id: Option<TurnRunId>,
    pub reply_target: ReplyTargetBindingRef,
    pub kind: OutboundPushKind,
    pub projection_ref: ProjectionUpdateRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionViewClass {
    ProductThread,
    ProductMission,
    ProductRun,
    DeliveryStatus,
    DebugSupport,
    AdminAudit,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionTarget {
    Thread { thread_id: ThreadId },
    Mission { mission_id: MissionId },
    Run { invocation_id: InvocationId },
    Process { process_id: ProcessId },
    DeliveryStatus { thread_id: ThreadId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionStreamItem {
    Snapshot(ProductProjectionEnvelope),
    Update(ProductProjectionEnvelope),
    RebaseRequired {
        snapshot: Box<ProductProjectionEnvelope>,
        rebased_from: Option<ProjectionCursor>,
        snapshot_cursor: ProjectionCursor,
    },
    Lagged {
        reason: LagReason,
        snapshot_cursor: ProjectionCursor,
    },
    KeepAlive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LagReason {
    SourceLagged,
    SourceFailed,
    SubscriberBackpressure,
    RedactionBlocked,
    AccessBlocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductProjectionEnvelope {
    ThreadSnapshot(ProjectionSnapshot),
    ThreadUpdates(ProjectionReplay),
    DeliveryStatus(DeliveryStatusProjectionPayload),
    Debug(DebugProjectionPayload),
}

impl ProductProjectionEnvelope {
    pub fn cursor(&self) -> ProjectionCursor {
        match self {
            Self::ThreadSnapshot(snapshot) => snapshot.next_cursor.clone(),
            Self::ThreadUpdates(replay) => replay.next_cursor.clone(),
            Self::DeliveryStatus(payload) => payload.cursor.clone(),
            Self::Debug(payload) => payload.cursor.clone(),
        }
    }

    pub fn scope(&self) -> &ProjectionScope {
        match self {
            Self::ThreadSnapshot(snapshot) => &snapshot.next_cursor.scope,
            Self::ThreadUpdates(replay) => &replay.next_cursor.scope,
            Self::DeliveryStatus(payload) => &payload.cursor.scope,
            Self::Debug(payload) => &payload.cursor.scope,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryStatusProjectionPayload {
    pub cursor: ProjectionCursor,
    pub delivery_ref: ProjectionUpdateRef,
    pub status: DeliveryProjectionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryProjectionStatus {
    Pending,
    Delivered,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebugProjectionPayload {
    pub cursor: ProjectionCursor,
    pub redacted_summary: String,
}

pub struct ProjectionSubscription {
    receiver: mpsc::Receiver<ProjectionStreamItem>,
    _admission: ProjectionStreamAdmissionPermit,
}

impl std::fmt::Debug for ProjectionSubscription {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProjectionSubscription")
            .field("receiver", &"<bounded_projection_stream>")
            .field("admission", &"<projection_stream_admission_permit>")
            .finish()
    }
}

impl ProjectionSubscription {
    pub async fn next(&mut self) -> Option<ProjectionStreamItem> {
        self.receiver.recv().await
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionAccessRequest {
    pub actor: TurnActor,
    pub scope: ProjectionScope,
    pub view: ProjectionViewClass,
    pub target: ProjectionTarget,
}

#[async_trait]
pub trait ProjectionAccessPolicy: Send + Sync {
    async fn authorize(
        &self,
        request: ProjectionAccessRequest,
    ) -> Result<(), ProjectionStreamError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionStreamAdmissionRequest {
    pub actor: TurnActor,
    pub tenant_id: TenantId,
    pub scope: ProjectionScope,
    pub view: ProjectionViewClass,
    pub target: ProjectionTarget,
}

#[async_trait]
pub trait ProjectionStreamAdmissionPolicy: Send + Sync {
    async fn admit(
        &self,
        request: ProjectionStreamAdmissionRequest,
    ) -> Result<ProjectionStreamAdmissionPermit, ProjectionStreamError>;
}

pub struct ProjectionStreamAdmissionPermit {
    release: Option<AdmissionRelease>,
}

impl ProjectionStreamAdmissionPermit {
    pub fn detached() -> Self {
        Self { release: None }
    }
}

impl Drop for ProjectionStreamAdmissionPermit {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release.release();
        }
    }
}

struct AdmissionRelease {
    state: Arc<Mutex<AdmissionState>>,
    tenant_key: TenantAdmissionKey,
    actor_key: ActorAdmissionKey,
    scope_key: ScopeAdmissionKey,
}

impl AdmissionRelease {
    fn release(self) {
        if let Ok(mut state) = self.state.lock() {
            decrement(&mut state.by_tenant, &self.tenant_key);
            decrement(&mut state.by_actor, &self.actor_key);
            decrement(&mut state.by_scope, &self.scope_key);
            state.global = state.global.saturating_sub(1);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectionStreamLimits {
    pub per_tenant: usize,
    pub per_actor: usize,
    pub per_scope: usize,
    pub global: usize,
}

impl Default for ProjectionStreamLimits {
    fn default() -> Self {
        Self {
            per_tenant: 64,
            per_actor: 16,
            per_scope: 8,
            global: 512,
        }
    }
}

#[derive(Default)]
pub struct InMemoryProjectionStreamAdmissionPolicy {
    limits: ProjectionStreamLimits,
    state: Arc<Mutex<AdmissionState>>,
}

#[derive(Default)]
struct AdmissionState {
    global: usize,
    by_tenant: HashMap<TenantAdmissionKey, usize>,
    by_actor: HashMap<ActorAdmissionKey, usize>,
    by_scope: HashMap<ScopeAdmissionKey, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TenantAdmissionKey(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ActorAdmissionKey(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ScopeAdmissionKey {
    scope: ProjectionScopeKey,
    target: ProjectionTargetKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProjectionScopeKey {
    tenant_id: String,
    user_id: String,
    agent_id: Option<String>,
    project_id: Option<String>,
    mission_id: Option<String>,
    thread_id: Option<String>,
    process_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ProjectionTargetKey {
    Thread(String),
    Mission(String),
    Run(String),
    Process(String),
    DeliveryStatus(String),
}

impl InMemoryProjectionStreamAdmissionPolicy {
    pub fn new(limits: ProjectionStreamLimits) -> Self {
        Self {
            limits,
            state: Arc::new(Mutex::new(AdmissionState::default())),
        }
    }
}

#[async_trait]
impl ProjectionStreamAdmissionPolicy for InMemoryProjectionStreamAdmissionPolicy {
    async fn admit(
        &self,
        request: ProjectionStreamAdmissionRequest,
    ) -> Result<ProjectionStreamAdmissionPermit, ProjectionStreamError> {
        let tenant_key = TenantAdmissionKey(request.tenant_id.to_string());
        let actor_key = ActorAdmissionKey(request.actor.user_id.to_string());
        let scope_key = scope_key(&request.scope, &request.target);
        let mut state = self
            .state
            .lock()
            .map_err(|_| ProjectionStreamError::Source)?;
        if state.global >= self.limits.global
            || count(&state.by_tenant, &tenant_key) >= self.limits.per_tenant
            || count(&state.by_actor, &actor_key) >= self.limits.per_actor
            || count(&state.by_scope, &scope_key) >= self.limits.per_scope
        {
            return Err(ProjectionStreamError::AdmissionDenied);
        }
        state.global += 1;
        increment(&mut state.by_tenant, &tenant_key);
        increment(&mut state.by_actor, &actor_key);
        increment(&mut state.by_scope, &scope_key);
        Ok(ProjectionStreamAdmissionPermit {
            release: Some(AdmissionRelease {
                state: Arc::clone(&self.state),
                tenant_key,
                actor_key,
                scope_key,
            }),
        })
    }
}

#[derive(Default)]
pub struct AllowAllProjectionAccessPolicy;

#[async_trait]
impl ProjectionAccessPolicy for AllowAllProjectionAccessPolicy {
    async fn authorize(
        &self,
        _request: ProjectionAccessRequest,
    ) -> Result<(), ProjectionStreamError> {
        Ok(())
    }
}

#[async_trait]
pub trait ProjectionUpdateSource: Send + Sync {
    async fn subscribe(
        &self,
        request: ProjectionLiveUpdateRequest,
    ) -> Result<broadcast::Receiver<ProductProjectionEnvelope>, ProjectionStreamError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionLiveUpdateRequest {
    pub actor: TurnActor,
    pub scope: ProjectionScope,
    pub view: ProjectionViewClass,
    pub target: ProjectionTarget,
}

pub struct InMemoryProjectionUpdateSource {
    sender: broadcast::Sender<ProductProjectionEnvelope>,
}

impl InMemoryProjectionUpdateSource {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity.max(1));
        Self { sender }
    }

    pub fn publish(
        &self,
        envelope: ProductProjectionEnvelope,
    ) -> Result<usize, ProjectionStreamError> {
        self.sender
            .send(envelope)
            .map_err(|_| ProjectionStreamError::Source)
    }
}

#[async_trait]
impl ProjectionUpdateSource for InMemoryProjectionUpdateSource {
    async fn subscribe(
        &self,
        _request: ProjectionLiveUpdateRequest,
    ) -> Result<broadcast::Receiver<ProductProjectionEnvelope>, ProjectionStreamError> {
        Ok(self.sender.subscribe())
    }
}

pub trait ProjectionRedactionValidator: Send + Sync {
    fn validate(&self, envelope: &ProductProjectionEnvelope) -> Result<(), ProjectionStreamError>;
}

#[derive(Default)]
pub struct NoExposureProjectionRedactionValidator;

impl ProjectionRedactionValidator for NoExposureProjectionRedactionValidator {
    fn validate(&self, envelope: &ProductProjectionEnvelope) -> Result<(), ProjectionStreamError> {
        let rendered =
            serde_json::to_string(envelope).map_err(|_| ProjectionStreamError::Source)?;
        if NO_EXPOSURE_SENTINELS
            .iter()
            .any(|sentinel| rendered.contains(sentinel))
        {
            return Err(ProjectionStreamError::Redaction);
        }
        Ok(())
    }
}

const NO_EXPOSURE_SENTINELS: &[&str] = &[
    "RAW_PROMPT_SENTINEL",
    "TOOL_INPUT_SENTINEL",
    "TOOL_OUTPUT_SENTINEL",
    "SECRET_SENTINEL",
    "HOST_PATH_SENTINEL",
    "RAW_RUNTIME_OUTPUT_SENTINEL",
    "BACKEND_DIAGNOSTIC_SENTINEL",
    "RAW_PROVIDER_ERROR_SENTINEL",
    "INVOCATION_FINGERPRINT_SENTINEL",
    "APPROVAL_REASON_SENTINEL",
    "LEASE_MATERIAL_SENTINEL",
];

pub struct EventStreamManager {
    projection: Arc<dyn EventProjectionService>,
    access_policy: Arc<dyn ProjectionAccessPolicy>,
    admission_policy: Arc<dyn ProjectionStreamAdmissionPolicy>,
    update_source: Arc<dyn ProjectionUpdateSource>,
    redaction_validator: Arc<dyn ProjectionRedactionValidator>,
    outbound_store: Arc<dyn OutboundStateStore>,
    validation_cache: ProjectionValidationCache,
}

#[derive(Clone, Default)]
struct ProjectionValidationCache {
    allowed: Arc<Mutex<HashSet<ProjectionValidationCacheKey>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProjectionValidationCacheKey {
    variant: ProjectionEnvelopeKind,
    scope: ProjectionScopeKey,
    cursor: u64,
    payload: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ProjectionEnvelopeKind {
    ThreadSnapshot,
    ThreadUpdates,
    DeliveryStatus,
    Debug,
}

impl ProjectionValidationCache {
    fn validate(
        &self,
        validator: &dyn ProjectionRedactionValidator,
        envelope: &ProductProjectionEnvelope,
    ) -> Result<(), ProjectionStreamError> {
        let key = validation_cache_key(envelope)?;
        if self
            .allowed
            .lock()
            .map_err(|_| ProjectionStreamError::Source)?
            .contains(&key)
        {
            return Ok(());
        }

        validator.validate(envelope)?;
        let mut allowed = self
            .allowed
            .lock()
            .map_err(|_| ProjectionStreamError::Source)?;
        if allowed.len() >= MAX_VALIDATION_CACHE_ENTRIES {
            allowed.clear();
        }
        allowed.insert(key);
        Ok(())
    }
}

impl EventStreamManager {
    pub fn new<P, A, M, U, R, O>(
        projection: Arc<P>,
        access_policy: Arc<A>,
        admission_policy: Arc<M>,
        update_source: Arc<U>,
        redaction_validator: Arc<R>,
        outbound_store: Arc<O>,
    ) -> Self
    where
        P: EventProjectionService + 'static,
        A: ProjectionAccessPolicy + 'static,
        M: ProjectionStreamAdmissionPolicy + 'static,
        U: ProjectionUpdateSource + 'static,
        R: ProjectionRedactionValidator + 'static,
        O: OutboundStateStore + 'static,
    {
        Self {
            projection,
            access_policy,
            admission_policy,
            update_source,
            redaction_validator,
            outbound_store,
            validation_cache: ProjectionValidationCache::default(),
        }
    }

    pub fn from_services(
        projection: Arc<dyn EventProjectionService>,
        access_policy: Arc<dyn ProjectionAccessPolicy>,
        admission_policy: Arc<dyn ProjectionStreamAdmissionPolicy>,
        update_source: Arc<dyn ProjectionUpdateSource>,
        redaction_validator: Arc<dyn ProjectionRedactionValidator>,
        outbound_store: Arc<dyn OutboundStateStore>,
    ) -> Self {
        Self {
            projection,
            access_policy,
            admission_policy,
            update_source,
            redaction_validator,
            outbound_store,
            validation_cache: ProjectionValidationCache::default(),
        }
    }

    pub async fn fetch_snapshot(
        &self,
        request: ProjectionFetchRequest,
    ) -> Result<ProjectionFetchResponse, ProjectionStreamError> {
        self.authorize(
            &request.actor,
            &request.scope,
            request.view,
            &request.target,
        )
        .await?;
        validate_product_thread_view(request.view, &request.target, &request.scope)?;
        let snapshot = self
            .projection
            .snapshot(ProjectionRequest {
                scope: request.scope.clone(),
                after: None,
                limit: request.limit,
            })
            .await
            .map_err(map_projection_error)?;
        let envelope = ProductProjectionEnvelope::ThreadSnapshot(snapshot);
        validate_stream_envelope(&envelope, request.view, &request.target, &request.scope)?;
        self.validation_cache
            .validate(self.redaction_validator.as_ref(), &envelope)?;
        Ok(ProjectionFetchResponse {
            cursor: envelope.cursor(),
            snapshot: envelope,
        })
    }

    pub async fn subscribe(
        &self,
        request: ProjectionSubscribeRequest,
    ) -> Result<ProjectionSubscription, ProjectionStreamError> {
        self.authorize(
            &request.actor,
            &request.scope,
            request.view,
            &request.target,
        )
        .await?;
        validate_product_thread_view(request.view, &request.target, &request.scope)?;
        let admission = self
            .admission_policy
            .admit(ProjectionStreamAdmissionRequest {
                actor: request.actor.clone(),
                tenant_id: request.scope.stream.tenant_id.clone(),
                scope: request.scope.clone(),
                view: request.view,
                target: request.target.clone(),
            })
            .await?;

        let live = self
            .update_source
            .subscribe(ProjectionLiveUpdateRequest {
                actor: request.actor.clone(),
                scope: request.scope.clone(),
                view: request.view,
                target: request.target.clone(),
            })
            .await?;

        let mut initial_items = Vec::new();
        let live_floor_cursor = match request.after_cursor.clone() {
            None => {
                let snapshot_envelope = self
                    .snapshot_envelope(&request.scope, request.limit)
                    .await?;
                validate_stream_envelope(
                    &snapshot_envelope,
                    request.view,
                    &request.target,
                    &request.scope,
                )?;
                self.validation_cache
                    .validate(self.redaction_validator.as_ref(), &snapshot_envelope)?;
                let cursor = snapshot_envelope.cursor();
                initial_items.push(ProjectionStreamItem::Snapshot(snapshot_envelope));
                cursor
            }
            Some(cursor) if cursor.scope != request.scope => {
                return Err(ProjectionStreamError::AccessDenied);
            }
            Some(cursor) => match self
                .projection
                .updates(ProjectionRequest {
                    scope: request.scope.clone(),
                    after: Some(cursor.clone()),
                    limit: request.limit,
                })
                .await
            {
                Ok(replay) => {
                    let update_envelope = ProductProjectionEnvelope::ThreadUpdates(replay);
                    validate_stream_envelope(
                        &update_envelope,
                        request.view,
                        &request.target,
                        &request.scope,
                    )?;
                    self.validation_cache
                        .validate(self.redaction_validator.as_ref(), &update_envelope)?;
                    let cursor = update_envelope.cursor();
                    initial_items.push(ProjectionStreamItem::Update(update_envelope));
                    cursor
                }
                Err(ProjectionError::RebaseRequired { .. }) => {
                    let snapshot_envelope = self
                        .snapshot_envelope(&request.scope, request.limit)
                        .await?;
                    validate_stream_envelope(
                        &snapshot_envelope,
                        request.view,
                        &request.target,
                        &request.scope,
                    )?;
                    self.validation_cache
                        .validate(self.redaction_validator.as_ref(), &snapshot_envelope)?;
                    let snapshot_cursor = snapshot_envelope.cursor();
                    initial_items.push(ProjectionStreamItem::RebaseRequired {
                        snapshot_cursor: snapshot_cursor.clone(),
                        snapshot: Box::new(snapshot_envelope),
                        rebased_from: Some(cursor.clone()),
                    });
                    snapshot_cursor
                }
                Err(error) => return Err(map_projection_error(error)),
            },
        };

        let capacity = request.capabilities.bounded_buffer_capacity()?;
        let (sender, receiver) = mpsc::channel(capacity);
        let redaction_validator = Arc::clone(&self.redaction_validator);
        let validation_cache = self.validation_cache.clone();
        tokio::spawn(forward_subscription_items(
            sender,
            initial_items,
            live,
            SubscriptionForwardContext {
                scope: request.scope,
                view: request.view,
                target: request.target,
                live_floor_cursor,
                redaction_validator,
                validation_cache,
            },
        ));
        Ok(ProjectionSubscription {
            receiver,
            _admission: admission,
        })
    }

    pub async fn push_candidates_for_update(
        &self,
        request: PushCandidatesForUpdateRequest,
    ) -> Result<Vec<OutboundPushCandidate>, ProjectionStreamError> {
        self.outbound_store
            .plan_push_targets(OutboundPushTargetRequest {
                scope: request.scope,
                turn_run_id: request.turn_run_id,
                reply_target: request.reply_target,
                kind: request.kind,
                projection_ref: request.projection_ref,
            })
            .await
            .map(|plan| plan.candidates)
            .map_err(map_outbound_error)
    }

    async fn authorize(
        &self,
        actor: &TurnActor,
        scope: &ProjectionScope,
        view: ProjectionViewClass,
        target: &ProjectionTarget,
    ) -> Result<(), ProjectionStreamError> {
        self.access_policy
            .authorize(ProjectionAccessRequest {
                actor: actor.clone(),
                scope: scope.clone(),
                view,
                target: target.clone(),
            })
            .await
    }

    async fn snapshot_envelope(
        &self,
        scope: &ProjectionScope,
        limit: usize,
    ) -> Result<ProductProjectionEnvelope, ProjectionStreamError> {
        self.projection
            .snapshot(ProjectionRequest {
                scope: scope.clone(),
                after: None,
                limit,
            })
            .await
            .map(ProductProjectionEnvelope::ThreadSnapshot)
            .map_err(map_projection_error)
    }
}

impl std::fmt::Debug for EventStreamManager {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EventStreamManager")
            .field("projection", &"<event_projection_service>")
            .field("access_policy", &"<projection_access_policy>")
            .field("admission_policy", &"<projection_stream_admission_policy>")
            .field("update_source", &"<projection_update_source>")
            .field("redaction_validator", &"<projection_redaction_validator>")
            .field("outbound_store", &"<outbound_state_store>")
            .field("validation_cache", &"<projection_validation_cache>")
            .finish()
    }
}

struct SubscriptionForwardContext {
    scope: ProjectionScope,
    view: ProjectionViewClass,
    target: ProjectionTarget,
    live_floor_cursor: ProjectionCursor,
    redaction_validator: Arc<dyn ProjectionRedactionValidator>,
    validation_cache: ProjectionValidationCache,
}

async fn forward_subscription_items(
    sender: mpsc::Sender<ProjectionStreamItem>,
    initial_items: Vec<ProjectionStreamItem>,
    mut live: broadcast::Receiver<ProductProjectionEnvelope>,
    context: SubscriptionForwardContext,
) {
    for item in initial_items {
        if sender.send(item).await.is_err() {
            return;
        }
    }

    let mut last_delivered_cursor = context.live_floor_cursor;
    loop {
        let received = tokio::select! {
            _ = sender.closed() => return,
            received = live.recv() => received,
        };
        match received {
            Ok(envelope) => {
                if envelope.scope() != &context.scope {
                    continue;
                }
                let envelope_cursor = envelope.cursor();
                if envelope_cursor.runtime <= last_delivered_cursor.runtime {
                    continue;
                }
                if validate_stream_envelope(
                    &envelope,
                    context.view,
                    &context.target,
                    &context.scope,
                )
                .is_err()
                {
                    send_terminal_lag(&sender, LagReason::AccessBlocked, &last_delivered_cursor)
                        .await;
                    return;
                }
                match context
                    .validation_cache
                    .validate(context.redaction_validator.as_ref(), &envelope)
                {
                    Ok(()) => {}
                    Err(ProjectionStreamError::Redaction) => {
                        send_terminal_lag(
                            &sender,
                            LagReason::RedactionBlocked,
                            &last_delivered_cursor,
                        )
                        .await;
                        return;
                    }
                    Err(_) => {
                        send_terminal_lag(&sender, LagReason::SourceFailed, &last_delivered_cursor)
                            .await;
                        return;
                    }
                }
                match sender.try_send(ProjectionStreamItem::Update(envelope)) {
                    Ok(()) => {
                        last_delivered_cursor = envelope_cursor;
                    }
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        send_terminal_lag(
                            &sender,
                            LagReason::SubscriberBackpressure,
                            &last_delivered_cursor,
                        )
                        .await;
                        return;
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => return,
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {
                send_terminal_lag(&sender, LagReason::SourceLagged, &last_delivered_cursor).await;
                return;
            }
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

async fn send_terminal_lag(
    sender: &mpsc::Sender<ProjectionStreamItem>,
    reason: LagReason,
    snapshot_cursor: &ProjectionCursor,
) {
    let item = ProjectionStreamItem::Lagged {
        reason,
        snapshot_cursor: snapshot_cursor.clone(),
    };
    match sender.try_send(item) {
        Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => {}
        Err(mpsc::error::TrySendError::Full(item)) => {
            tokio::select! {
                _ = sender.closed() => {}
                _ = sleep(Duration::from_millis(TERMINAL_LAG_SEND_TIMEOUT_MILLIS)) => {}
                result = sender.send(item) => {
                    let _ = result;
                }
            }
        }
    }
}

fn validate_stream_envelope(
    envelope: &ProductProjectionEnvelope,
    view: ProjectionViewClass,
    target: &ProjectionTarget,
    scope: &ProjectionScope,
) -> Result<(), ProjectionStreamError> {
    if envelope.scope() != scope {
        return Err(ProjectionStreamError::AccessDenied);
    }
    match (view, target, envelope) {
        (
            ProjectionViewClass::ProductThread,
            ProjectionTarget::Thread { thread_id },
            ProductProjectionEnvelope::ThreadSnapshot(_)
            | ProductProjectionEnvelope::ThreadUpdates(_),
        ) if scope.read_scope.thread_id.as_ref() == Some(thread_id) => {
            validate_product_thread_payload(envelope, thread_id)
        }
        _ => Err(ProjectionStreamError::AccessDenied),
    }
}

fn validate_product_thread_payload(
    envelope: &ProductProjectionEnvelope,
    thread_id: &ThreadId,
) -> Result<(), ProjectionStreamError> {
    let all_thread_entries_match = |entries: &[ironclaw_event_projections::TimelineEntry]| {
        entries
            .iter()
            .all(|entry| entry.thread_id.as_ref() == Some(thread_id))
    };
    let all_run_statuses_match = |runs: &[ironclaw_event_projections::RunStatusProjection]| {
        runs.iter()
            .all(|run| run.thread_id.as_ref() == Some(thread_id))
    };

    match envelope {
        ProductProjectionEnvelope::ThreadSnapshot(snapshot) => {
            if all_thread_entries_match(&snapshot.timeline.entries)
                && all_run_statuses_match(&snapshot.runs)
            {
                Ok(())
            } else {
                Err(ProjectionStreamError::AccessDenied)
            }
        }
        ProductProjectionEnvelope::ThreadUpdates(replay) => {
            if all_thread_entries_match(&replay.updates) && all_run_statuses_match(&replay.runs) {
                Ok(())
            } else {
                Err(ProjectionStreamError::AccessDenied)
            }
        }
        ProductProjectionEnvelope::DeliveryStatus(_) | ProductProjectionEnvelope::Debug(_) => {
            Err(ProjectionStreamError::AccessDenied)
        }
    }
}

fn validate_product_thread_view(
    view: ProjectionViewClass,
    target: &ProjectionTarget,
    scope: &ProjectionScope,
) -> Result<(), ProjectionStreamError> {
    match (view, target) {
        (ProjectionViewClass::ProductThread, ProjectionTarget::Thread { thread_id }) => {
            if scope.read_scope.thread_id.as_ref() == Some(thread_id) {
                Ok(())
            } else {
                Err(ProjectionStreamError::AccessDenied)
            }
        }
        (ProjectionViewClass::DebugSupport | ProjectionViewClass::AdminAudit, _) => {
            Err(ProjectionStreamError::AccessDenied)
        }
        _ => Err(ProjectionStreamError::InvalidRequest {
            reason: "projection view/target is not implemented in the first EventStreamManager slice",
        }),
    }
}

fn map_projection_error(error: ProjectionError) -> ProjectionStreamError {
    match error {
        ProjectionError::InvalidRequest { reason } => {
            ProjectionStreamError::InvalidRequest { reason }
        }
        ProjectionError::RebaseRequired { .. } => ProjectionStreamError::InvalidRequest {
            reason: "projection rebase required outside subscribe flow",
        },
        ProjectionError::Source { .. } => ProjectionStreamError::Source,
    }
}

fn map_outbound_error(error: OutboundError) -> ProjectionStreamError {
    match error {
        OutboundError::AccessDenied => ProjectionStreamError::AccessDenied,
        OutboundError::InvalidRequest { reason } => {
            ProjectionStreamError::InvalidRequest { reason }
        }
        OutboundError::Backend
        | OutboundError::CasConflict
        | OutboundError::Serialization
        | OutboundError::SubscriptionScopeMismatch
        | OutboundError::DeliveryNotFound => ProjectionStreamError::Outbound,
    }
}

fn scope_key(scope: &ProjectionScope, target: &ProjectionTarget) -> ScopeAdmissionKey {
    ScopeAdmissionKey {
        scope: projection_scope_key(scope),
        target: target_key(target),
    }
}

fn projection_scope_key(scope: &ProjectionScope) -> ProjectionScopeKey {
    ProjectionScopeKey {
        tenant_id: scope.stream.tenant_id.to_string(),
        user_id: scope.stream.user_id.to_string(),
        agent_id: scope.stream.agent_id.as_ref().map(ToString::to_string),
        project_id: scope
            .read_scope
            .project_id
            .as_ref()
            .map(ToString::to_string),
        mission_id: scope
            .read_scope
            .mission_id
            .as_ref()
            .map(ToString::to_string),
        thread_id: scope.read_scope.thread_id.as_ref().map(ToString::to_string),
        process_id: scope
            .read_scope
            .process_id
            .as_ref()
            .map(ToString::to_string),
    }
}

fn target_key(target: &ProjectionTarget) -> ProjectionTargetKey {
    match target {
        ProjectionTarget::Thread { thread_id } => {
            ProjectionTargetKey::Thread(thread_id.to_string())
        }
        ProjectionTarget::Mission { mission_id } => {
            ProjectionTargetKey::Mission(mission_id.to_string())
        }
        ProjectionTarget::Run { invocation_id } => {
            ProjectionTargetKey::Run(invocation_id.to_string())
        }
        ProjectionTarget::Process { process_id } => {
            ProjectionTargetKey::Process(process_id.to_string())
        }
        ProjectionTarget::DeliveryStatus { thread_id } => {
            ProjectionTargetKey::DeliveryStatus(thread_id.to_string())
        }
    }
}

fn validation_cache_key(
    envelope: &ProductProjectionEnvelope,
) -> Result<ProjectionValidationCacheKey, ProjectionStreamError> {
    let variant = match envelope {
        ProductProjectionEnvelope::ThreadSnapshot(_) => ProjectionEnvelopeKind::ThreadSnapshot,
        ProductProjectionEnvelope::ThreadUpdates(_) => ProjectionEnvelopeKind::ThreadUpdates,
        ProductProjectionEnvelope::DeliveryStatus(_) => ProjectionEnvelopeKind::DeliveryStatus,
        ProductProjectionEnvelope::Debug(_) => ProjectionEnvelopeKind::Debug,
    };
    let payload = serde_json::to_vec(envelope).map_err(|_| ProjectionStreamError::Source)?;
    Ok(ProjectionValidationCacheKey {
        variant,
        scope: projection_scope_key(envelope.scope()),
        cursor: envelope.cursor().runtime.as_u64(),
        payload,
    })
}

fn count<K>(map: &HashMap<K, usize>, key: &K) -> usize
where
    K: Eq + Hash,
{
    map.get(key).copied().unwrap_or(0)
}

fn increment<K>(map: &mut HashMap<K, usize>, key: &K)
where
    K: Clone + Eq + Hash,
{
    *map.entry(key.clone()).or_insert(0) += 1;
}

fn decrement<K>(map: &mut HashMap<K, usize>, key: &K)
where
    K: Eq + Hash,
{
    if let Some(value) = map.get_mut(key) {
        *value = value.saturating_sub(1);
        if *value == 0 {
            map.remove(key);
        }
    }
}

pub fn keep_alive_item() -> ProjectionStreamItem {
    ProjectionStreamItem::KeepAlive
}
