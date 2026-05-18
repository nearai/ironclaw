//! Transport-neutral Reborn projection streams.
//!
//! This crate composes product-safe projection DTOs with access, admission,
//! live-update, redaction, and outbound-candidate seams. It intentionally does
//! not render SSE/WebSocket/channel frames and does not read durable logs
//! directly.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
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
use thiserror::Error;
use tokio::sync::{broadcast, mpsc};

const DEFAULT_SUBSCRIPTION_BUFFER: usize = 16;
const MIN_SUBSCRIPTION_BUFFER: usize = 1;

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
    fn bounded_buffer_capacity(&self) -> usize {
        self.buffer_capacity.max(MIN_SUBSCRIPTION_BUFFER)
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
    SubscriberBackpressure,
    RedactionBlocked,
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

#[derive(Debug, Error)]
pub enum ProjectionStreamError {
    #[error("projection stream request rejected: {reason}")]
    InvalidRequest { reason: &'static str },
    #[error("projection stream access denied")]
    AccessDenied,
    #[error("projection stream admission denied")]
    AdmissionDenied,
    #[error("projection stream source failed")]
    Source,
    #[error("projection stream payload failed redaction validation")]
    Redaction,
    #[error("projection stream outbound policy failed")]
    Outbound,
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
    tenant_key: String,
    actor_key: String,
    scope_key: String,
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
    by_tenant: HashMap<String, usize>,
    by_actor: HashMap<String, usize>,
    by_scope: HashMap<String, usize>,
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
        let tenant_key = request.tenant_id.to_string();
        let actor_key = request.actor.user_id.to_string();
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
                scope: request.scope,
                after: None,
                limit: request.limit,
            })
            .await
            .map_err(map_projection_error)?;
        let envelope = ProductProjectionEnvelope::ThreadSnapshot(snapshot);
        self.redaction_validator.validate(&envelope)?;
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

        let mut initial_items = Vec::new();
        let snapshot = self
            .projection
            .snapshot(ProjectionRequest {
                scope: request.scope.clone(),
                after: None,
                limit: request.limit,
            })
            .await
            .map_err(map_projection_error)?;
        let snapshot_envelope = ProductProjectionEnvelope::ThreadSnapshot(snapshot);
        self.redaction_validator.validate(&snapshot_envelope)?;

        match request.after_cursor.clone() {
            None => initial_items.push(ProjectionStreamItem::Snapshot(snapshot_envelope.clone())),
            Some(cursor) if cursor.scope != request.scope => {
                initial_items.push(ProjectionStreamItem::RebaseRequired {
                    snapshot_cursor: snapshot_envelope.cursor(),
                    snapshot: Box::new(snapshot_envelope.clone()),
                    rebased_from: None,
                });
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
                    initial_items.push(ProjectionStreamItem::Snapshot(snapshot_envelope.clone()));
                    let update_envelope = ProductProjectionEnvelope::ThreadUpdates(replay);
                    self.redaction_validator.validate(&update_envelope)?;
                    initial_items.push(ProjectionStreamItem::Update(update_envelope));
                }
                Err(ProjectionError::RebaseRequired { .. }) => {
                    initial_items.push(ProjectionStreamItem::RebaseRequired {
                        snapshot_cursor: snapshot_envelope.cursor(),
                        snapshot: Box::new(snapshot_envelope.clone()),
                        rebased_from: Some(cursor),
                    });
                }
                Err(error) => return Err(map_projection_error(error)),
            },
        }

        let live = self
            .update_source
            .subscribe(ProjectionLiveUpdateRequest {
                actor: request.actor,
                scope: request.scope.clone(),
                view: request.view,
                target: request.target,
            })
            .await?;
        let capacity = request.capabilities.bounded_buffer_capacity();
        let (sender, receiver) = mpsc::channel(capacity);
        let redaction_validator = Arc::clone(&self.redaction_validator);
        tokio::spawn(forward_subscription_items(
            sender,
            initial_items,
            live,
            request.scope,
            snapshot_envelope.cursor(),
            redaction_validator,
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
            .finish()
    }
}

async fn forward_subscription_items(
    sender: mpsc::Sender<ProjectionStreamItem>,
    initial_items: Vec<ProjectionStreamItem>,
    mut live: broadcast::Receiver<ProductProjectionEnvelope>,
    scope: ProjectionScope,
    snapshot_cursor: ProjectionCursor,
    redaction_validator: Arc<dyn ProjectionRedactionValidator>,
) {
    for item in initial_items {
        if sender.send(item).await.is_err() {
            return;
        }
    }

    loop {
        match live.recv().await {
            Ok(envelope) => {
                if envelope.scope() != &scope {
                    continue;
                }
                if redaction_validator.validate(&envelope).is_err() {
                    let _ = sender
                        .send(ProjectionStreamItem::Lagged {
                            reason: LagReason::RedactionBlocked,
                            snapshot_cursor: snapshot_cursor.clone(),
                        })
                        .await;
                    return;
                }
                if sender
                    .send(ProjectionStreamItem::Update(envelope))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {
                let _ = sender
                    .send(ProjectionStreamItem::Lagged {
                        reason: LagReason::SourceLagged,
                        snapshot_cursor: snapshot_cursor.clone(),
                    })
                    .await;
                return;
            }
            Err(broadcast::error::RecvError::Closed) => return,
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

fn scope_key(scope: &ProjectionScope, target: &ProjectionTarget) -> String {
    format!("{scope:?}:{target:?}")
}

fn count(map: &HashMap<String, usize>, key: &str) -> usize {
    map.get(key).copied().unwrap_or(0)
}

fn increment(map: &mut HashMap<String, usize>, key: &str) {
    *map.entry(key.to_string()).or_insert(0) += 1;
}

fn decrement(map: &mut HashMap<String, usize>, key: &str) {
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
