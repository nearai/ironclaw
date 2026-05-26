use std::sync::Arc;

use ironclaw_event_projections::{
    EventCursor, EventProjectionService, ProjectionCursor, ProjectionError, ProjectionRequest,
    ProjectionScope, ProjectionSnapshot,
};
use ironclaw_host_api::ThreadId;
use ironclaw_outbound::{
    OutboundError, OutboundPushCandidate, OutboundPushTargetRequest, OutboundStateStore,
};
use ironclaw_turns::TurnActor;
use tokio::sync::{broadcast, mpsc};

use crate::{
    admission::{
        ProjectionAccessPolicy, ProjectionAccessRequest, ProjectionStreamAdmissionPolicy,
        ProjectionStreamAdmissionRequest,
    },
    error::ProjectionStreamError,
    redaction::{ProjectionRedactionValidator, ProjectionValidationCache},
    types::{
        LagReason, ProductProjectionEnvelope, ProjectionFetchRequest, ProjectionFetchResponse,
        ProjectionStreamItem, ProjectionSubscribeRequest, ProjectionSubscription, ProjectionTarget,
        ProjectionViewClass, PushCandidatesForUpdateRequest,
    },
    update_source::{ProjectionLiveUpdateRequest, ProjectionUpdateSource},
};

pub struct EventStreamManager {
    projection: Arc<dyn EventProjectionService>,
    access_policy: Arc<dyn ProjectionAccessPolicy>,
    admission_policy: Arc<dyn ProjectionStreamAdmissionPolicy>,
    update_source: Arc<dyn ProjectionUpdateSource>,
    redaction_validator: Arc<dyn ProjectionRedactionValidator>,
    outbound_store: Arc<dyn OutboundStateStore>,
    validation_cache: ProjectionValidationCache,
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
        validate_actor_stream_user(&request.actor, request.view, &request.scope)?;
        validate_product_thread_view(request.view, &request.target, &request.scope)?;
        let envelope = self
            .snapshot_envelope(&request.scope, request.limit, true)
            .await?;
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
        validate_actor_stream_user(&request.actor, request.view, &request.scope)?;
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
        let mut initial_terminal_item = None;
        let live_floor_cursor = match request.after_cursor.clone() {
            None => {
                let snapshot_envelope = self
                    .snapshot_envelope(&request.scope, request.limit, true)
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
                if envelope_is_truncated(&snapshot_envelope) {
                    initial_terminal_item = Some(truncated_lag_item(&cursor));
                }
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
                    let update_envelope =
                        Arc::new(ProductProjectionEnvelope::ThreadUpdates(replay));
                    validate_stream_envelope(
                        update_envelope.as_ref(),
                        request.view,
                        &request.target,
                        &request.scope,
                    )?;
                    self.validation_cache
                        .validate(self.redaction_validator.as_ref(), update_envelope.as_ref())?;
                    let cursor = update_envelope.cursor();
                    if envelope_is_truncated(update_envelope.as_ref()) {
                        initial_terminal_item = Some(truncated_lag_item(&cursor));
                    }
                    initial_items.push(ProjectionStreamItem::Update(update_envelope));
                    cursor
                }
                Err(ProjectionError::RebaseRequired { .. }) => {
                    let snapshot_envelope = self
                        .snapshot_envelope(&request.scope, request.limit, false)
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
                    if envelope_is_truncated(&snapshot_envelope) {
                        initial_terminal_item = Some(truncated_lag_item(&snapshot_cursor));
                    }
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
        let (terminal_sender, terminal_receiver) = mpsc::channel(1);
        let redaction_validator = Arc::clone(&self.redaction_validator);
        let validation_cache = self.validation_cache.clone();
        tokio::spawn(forward_subscription_items(
            sender,
            terminal_sender,
            initial_items,
            initial_terminal_item,
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
        Ok(ProjectionSubscription::new(
            receiver,
            terminal_receiver,
            admission,
        ))
    }

    pub async fn push_candidates_for_update(
        &self,
        request: PushCandidatesForUpdateRequest,
    ) -> Result<Vec<OutboundPushCandidate>, ProjectionStreamError> {
        self.authorize(
            &request.actor,
            &request.projection_scope,
            request.view,
            &request.target,
        )
        .await?;
        validate_actor_stream_user(&request.actor, request.view, &request.projection_scope)?;
        validate_product_thread_view(request.view, &request.target, &request.projection_scope)?;
        validate_push_scope_matches_projection(&request)?;
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
        allow_rebase_baseline: bool,
    ) -> Result<ProductProjectionEnvelope, ProjectionStreamError> {
        let after = None;
        self.snapshot_envelope_after(scope, limit, after, allow_rebase_baseline)
            .await
    }

    async fn snapshot_envelope_after(
        &self,
        scope: &ProjectionScope,
        limit: usize,
        after: Option<ProjectionCursor>,
        allow_rebase_baseline: bool,
    ) -> Result<ProductProjectionEnvelope, ProjectionStreamError> {
        match self.load_snapshot(scope, limit, after).await {
            Ok(snapshot) => Ok(ProductProjectionEnvelope::ThreadSnapshot(snapshot)),
            Err(ProjectionError::RebaseRequired { earliest, .. }) if allow_rebase_baseline => {
                self.snapshot_envelope_at_rebase_cursor(scope, limit, *earliest)
                    .await
            }
            Err(error) => Err(map_projection_error(error)),
        }
    }

    async fn load_snapshot(
        &self,
        scope: &ProjectionScope,
        limit: usize,
        after: Option<ProjectionCursor>,
    ) -> Result<ProjectionSnapshot, ProjectionError> {
        self.projection
            .snapshot(ProjectionRequest {
                scope: scope.clone(),
                after,
                limit,
            })
            .await
    }

    async fn snapshot_envelope_at_rebase_cursor(
        &self,
        scope: &ProjectionScope,
        limit: usize,
        earliest: ProjectionCursor,
    ) -> Result<ProductProjectionEnvelope, ProjectionStreamError> {
        if earliest.scope != *scope {
            return Err(ProjectionStreamError::InvalidRequest {
                reason: "projection rebase cursor scope mismatch",
            });
        }
        let before_earliest = ProjectionCursor::for_scope(
            earliest.scope,
            EventCursor::new(earliest.runtime.as_u64().saturating_sub(1)),
        );
        self.load_snapshot(scope, limit, Some(before_earliest))
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
    terminal_sender: mpsc::Sender<ProjectionStreamItem>,
    initial_items: Vec<ProjectionStreamItem>,
    initial_terminal_item: Option<ProjectionStreamItem>,
    mut live: broadcast::Receiver<Arc<ProductProjectionEnvelope>>,
    context: SubscriptionForwardContext,
) {
    for item in initial_items {
        if sender.send(item).await.is_err() {
            return;
        }
    }
    if let Some(item) = initial_terminal_item {
        let _ = sender.send(item).await;
        return;
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
                    envelope.as_ref(),
                    context.view,
                    &context.target,
                    &context.scope,
                )
                .is_err()
                {
                    send_terminal_lag(
                        &terminal_sender,
                        LagReason::AccessBlocked,
                        &last_delivered_cursor,
                    );
                    return;
                }
                match context
                    .validation_cache
                    .validate_shared(context.redaction_validator.as_ref(), &envelope)
                {
                    Ok(()) => {}
                    Err(ProjectionStreamError::Redaction) => {
                        send_terminal_lag(
                            &terminal_sender,
                            LagReason::RedactionBlocked,
                            &last_delivered_cursor,
                        );
                        return;
                    }
                    Err(_) => {
                        send_terminal_lag(
                            &terminal_sender,
                            LagReason::SourceFailed,
                            &last_delivered_cursor,
                        );
                        return;
                    }
                }
                match sender.try_send(ProjectionStreamItem::Update(envelope)) {
                    Ok(()) => {
                        last_delivered_cursor = envelope_cursor;
                    }
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        send_terminal_lag(
                            &terminal_sender,
                            LagReason::SubscriberBackpressure,
                            &last_delivered_cursor,
                        );
                        return;
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => return,
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {
                send_terminal_lag(
                    &terminal_sender,
                    LagReason::SourceLagged,
                    &last_delivered_cursor,
                );
                return;
            }
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

fn truncated_lag_item(snapshot_cursor: &ProjectionCursor) -> ProjectionStreamItem {
    ProjectionStreamItem::Lagged {
        reason: LagReason::SourceLagged,
        snapshot_cursor: snapshot_cursor.clone(),
    }
}

fn envelope_is_truncated(envelope: &ProductProjectionEnvelope) -> bool {
    match envelope {
        ProductProjectionEnvelope::ThreadSnapshot(snapshot) => snapshot.truncated,
        ProductProjectionEnvelope::ThreadUpdates(replay) => replay.truncated,
        ProductProjectionEnvelope::DeliveryStatus(_) | ProductProjectionEnvelope::Debug(_) => false,
    }
}

fn send_terminal_lag(
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
        Err(mpsc::error::TrySendError::Full(_)) => {}
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
    let all_capability_activities_match =
        |activities: &[ironclaw_event_projections::CapabilityActivityProjection]| {
            activities
                .iter()
                .all(|activity| activity.thread_id.as_ref() == Some(thread_id))
        };

    match envelope {
        ProductProjectionEnvelope::ThreadSnapshot(snapshot) => {
            if all_thread_entries_match(&snapshot.timeline.entries)
                && all_run_statuses_match(&snapshot.runs)
                && all_capability_activities_match(&snapshot.capability_activities)
            {
                Ok(())
            } else {
                Err(ProjectionStreamError::AccessDenied)
            }
        }
        ProductProjectionEnvelope::ThreadUpdates(replay) => {
            if all_thread_entries_match(&replay.updates)
                && all_run_statuses_match(&replay.runs)
                && all_capability_activities_match(&replay.capability_activities)
                && all_capability_activities_match(&replay.capability_activity_transitions)
            {
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

fn validate_actor_stream_user(
    actor: &TurnActor,
    view: ProjectionViewClass,
    scope: &ProjectionScope,
) -> Result<(), ProjectionStreamError> {
    match view {
        ProjectionViewClass::ProductThread if actor.user_id != scope.stream.user_id => {
            Err(ProjectionStreamError::AccessDenied)
        }
        _ => Ok(()),
    }
}

fn validate_push_scope_matches_projection(
    request: &PushCandidatesForUpdateRequest,
) -> Result<(), ProjectionStreamError> {
    match &request.target {
        ProjectionTarget::Thread { thread_id }
            if request.scope.tenant_id == request.projection_scope.stream.tenant_id
                && request.scope.agent_id == request.projection_scope.stream.agent_id
                && request.scope.project_id == request.projection_scope.read_scope.project_id
                && Some(&request.scope.thread_id)
                    == request.projection_scope.read_scope.thread_id.as_ref()
                && request.projection_scope.read_scope.mission_id.is_none()
                && request.projection_scope.read_scope.process_id.is_none()
                && thread_id == &request.scope.thread_id =>
        {
            Ok(())
        }
        _ => Err(ProjectionStreamError::AccessDenied),
    }
}

fn map_projection_error(error: ProjectionError) -> ProjectionStreamError {
    match error {
        ProjectionError::InvalidRequest { reason } => {
            ProjectionStreamError::InvalidRequest { reason }
        }
        ProjectionError::MissingProjectionMetadata { .. } => {
            ProjectionStreamError::InvalidRequest {
                reason: "projection metadata missing on lifecycle event",
            }
        }
        ProjectionError::RebaseRequired { .. } => ProjectionStreamError::InvalidRequest {
            reason: "projection rebase required outside subscribe flow",
        },
        ProjectionError::TurnEventRebaseRequired { .. } => ProjectionStreamError::InvalidRequest {
            reason: "turn event projection rebase required outside subscribe flow",
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
