use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_event_projections::{
    CapabilityActivityProjection, CapabilityActivityStatus, EventProjectionService,
    ProjectionCursor as EventProjectionCursor, ProjectionReplay,
    ProjectionScope as EventProjectionScope, ProjectionSnapshot, ReplayEventProjectionService,
    RunProjectionStatus, RunStatusProjection,
};
use ironclaw_event_streams::{
    AllowAllProjectionAccessPolicy, EventStreamManager, InMemoryProjectionStreamAdmissionPolicy,
    InMemoryProjectionUpdateSource, NoExposureProjectionRedactionValidator,
    ProjectionStreamError as EventProjectionStreamError, ProjectionStreamItem,
    ProjectionSubscribeRequest, ProjectionTarget, ProjectionViewClass, SubscriberCapabilities,
};
use ironclaw_events::{DurableEventLog, EventCursor, EventStreamKey, ReadScope};
use ironclaw_outbound::InMemoryOutboundStateStore;
use ironclaw_product_adapters::{
    AdapterInstallationId, CapabilityActivityStatusView, CapabilityActivityView,
    CapabilityActivityViewInput, ExternalActorRef, ExternalConversationRef, ProductAdapterError,
    ProductAdapterId, ProductOutboundEnvelope, ProductOutboundPayload, ProductOutboundTarget,
    ProductProjectionItem, ProductProjectionState, ProductWorkflowRejectionKind,
    ProjectionCursor as ProductProjectionCursor, ProjectionStream, ProjectionSubscriptionRequest,
    RedactedString,
};
use ironclaw_turns::{
    ReplyTargetBindingRef, TurnActor, TurnCoordinator, TurnEventProjectionCursor,
    TurnEventProjectionSource, TurnRunId, TurnScope,
};

mod turn_events;
use turn_events::{TurnEventBridge, TurnEventPayload};

const WEBUI_PROJECTION_PAGE_LIMIT: usize = 256;
const WEBUI_RUNTIME_ITEM_MAX_PAYLOADS: usize = WEBUI_PROJECTION_PAGE_LIMIT + 1;
const WEBUI_PROJECTION_ADAPTER_ID: &str = "webui_v2";
const WEBUI_PROJECTION_INSTALLATION_ID: &str = "webui_v2.local";

#[derive(Clone)]
pub(crate) struct RebornProjectionServices {
    event_stream_manager: Arc<EventStreamManager>,
    turn_events: TurnEventBridge,
    webui_reply_target_binding_ref: ReplyTargetBindingRef,
}

impl RebornProjectionServices {
    pub(crate) fn with_turn_events(
        mut self,
        turn_event_source: Arc<dyn TurnEventProjectionSource>,
        turn_coordinator: Arc<dyn TurnCoordinator>,
    ) -> Self {
        self.turn_events = TurnEventBridge::enabled(turn_event_source, turn_coordinator);
        self
    }

    pub(crate) fn webui_event_stream(&self) -> Arc<dyn ProjectionStream> {
        Arc::new(WebuiRuntimeProjectionStream {
            manager: Arc::clone(&self.event_stream_manager),
            turn_events: self.turn_events.clone(),
            reply_target_binding_ref: self.webui_reply_target_binding_ref.clone(),
        })
    }
}

pub(crate) fn build_reborn_projection_services(
    event_log: Arc<dyn DurableEventLog>,
    webui_reply_target_binding_ref: ReplyTargetBindingRef,
) -> RebornProjectionServices {
    let projection: Arc<dyn EventProjectionService> =
        Arc::new(ReplayEventProjectionService::from_runtime_log(event_log));
    let event_stream_manager = Arc::new(EventStreamManager::from_services(
        projection,
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        Arc::new(InMemoryProjectionUpdateSource::new(128)),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    ));
    RebornProjectionServices {
        event_stream_manager,
        turn_events: TurnEventBridge::default(),
        webui_reply_target_binding_ref,
    }
}

/// WebUI bridge over the shared EventStreamManager.
///
/// This exposes runtime projection payloads that WebChat v2 has first-class
/// SSE frames for: run status and capability activity. Timeline content stays
/// behind the WebUI timeline facade until the browser event schema grows a
/// first-class timeline-entry mapper.
struct WebuiRuntimeProjectionStream {
    manager: Arc<EventStreamManager>,
    turn_events: TurnEventBridge,
    reply_target_binding_ref: ReplyTargetBindingRef,
}

#[async_trait]
impl ProjectionStream for WebuiRuntimeProjectionStream {
    async fn drain(
        &self,
        request: ProjectionSubscriptionRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, ProductAdapterError> {
        let projection_scope = runtime_projection_scope(&request.actor, &request.scope);
        let origin_cursor = request
            .after_cursor
            .map(|cursor| parse_webui_projection_cursor(cursor.as_str()))
            .transpose()?
            .unwrap_or_default();
        validate_webui_projection_cursor_scope(&origin_cursor, &request.scope, &projection_scope)?;
        let mut subscription = self
            .manager
            .subscribe(ProjectionSubscribeRequest {
                actor: request.actor.clone(),
                scope: projection_scope.clone(),
                view: ProjectionViewClass::ProductThread,
                target: ProjectionTarget::Thread {
                    thread_id: request.scope.thread_id.clone(),
                },
                after_cursor: origin_cursor.runtime.clone(),
                limit: WEBUI_PROJECTION_PAGE_LIMIT,
                capabilities: SubscriberCapabilities::default(),
            })
            .await
            .map_err(map_event_stream_error)?;

        let resumes_runtime_item = origin_cursor.runtime_payloads_delivered > 0;
        let mut batch = WebuiProjectionBatch::new(origin_cursor);
        if let Some(item) = subscription.next().await
            && batch.push_runtime_item(item, &request.scope)?
            && !resumes_runtime_item
        {
            for _ in 1..WEBUI_PROJECTION_PAGE_LIMIT {
                if !batch.has_runtime_payload_capacity() {
                    break;
                }
                let Some(item) = subscription.try_next_buffered() else {
                    break;
                };
                if !batch.push_runtime_item(item, &request.scope)? {
                    break;
                }
            }
        }

        if batch.runtime_payloads_pushed == 0 && !resumes_runtime_item {
            for _ in 0..WEBUI_PROJECTION_PAGE_LIMIT {
                if !batch.has_runtime_payload_capacity() {
                    break;
                }
                let Some(item) = subscription.try_next_buffered() else {
                    break;
                };
                if !batch.push_runtime_item(item, &request.scope)? {
                    break;
                }
            }
        }

        let turn_after = batch.cursor().turn.clone();
        let turn_drain = self.turn_events.drain(&request.scope, turn_after).await?;
        for TurnEventPayload {
            cursor: turn_cursor,
            payload,
        } in turn_drain.payloads
        {
            batch.push_turn(turn_cursor, payload);
        }
        if let Some(next_cursor) = turn_drain.next_cursor
            && batch.cursor().turn.as_ref() != Some(&next_cursor)
        {
            batch.push_turn(next_cursor, ProductOutboundPayload::KeepAlive);
        }

        batch
            .into_payloads()
            .map(|(cursor, payload)| {
                envelope_to_outbound(
                    product_cursor_from_webui_cursor(&cursor)?,
                    payload,
                    &request.scope,
                    &request.actor,
                    &self.reply_target_binding_ref,
                )
            })
            .collect()
    }
}

struct WebuiProjectionBatch {
    cursor: WebuiProjectionCursor,
    runtime_payloads_pushed: usize,
    payloads: Vec<(WebuiProjectionCursor, ProductOutboundPayload)>,
}

impl WebuiProjectionBatch {
    fn new(cursor: WebuiProjectionCursor) -> Self {
        Self {
            cursor,
            runtime_payloads_pushed: 0,
            payloads: Vec::new(),
        }
    }

    fn cursor(&self) -> &WebuiProjectionCursor {
        &self.cursor
    }

    fn push_runtime_payloads(
        &mut self,
        final_cursor: EventProjectionCursor,
        item_cursor: EventProjectionCursor,
        payloads: Vec<ProductOutboundPayload>,
        total: usize,
        already_delivered: usize,
    ) -> Result<bool, ProductAdapterError> {
        if total == 0 {
            return Ok(true);
        }

        if already_delivered > 0 && already_delivered >= total {
            return Err(ProductAdapterError::InvalidIdentifier {
                kind: "projection_cursor",
                reason: "runtime delivery offset exceeds runtime item payload count".to_string(),
            });
        }

        let remaining_capacity =
            WEBUI_RUNTIME_ITEM_MAX_PAYLOADS.saturating_sub(self.runtime_payloads_pushed);
        if remaining_capacity == 0 {
            return Ok(false);
        }

        for (index, payload) in payloads.into_iter().take(remaining_capacity).enumerate() {
            let delivered = already_delivered + index + 1;
            self.runtime_payloads_pushed += 1;
            if delivered == total {
                self.cursor.runtime = Some(max_projection_cursor(
                    final_cursor.clone(),
                    item_cursor.clone(),
                ));
                self.cursor.runtime_item = None;
                self.cursor.runtime_payloads_delivered = 0;
            } else {
                self.cursor.runtime_item = Some(item_cursor.runtime);
                self.cursor.runtime_payloads_delivered = delivered;
            }
            self.push(payload);
        }
        Ok(self.cursor.runtime_payloads_delivered == 0)
    }

    fn push_runtime_item(
        &mut self,
        item: ProjectionStreamItem,
        scope: &TurnScope,
    ) -> Result<bool, ProductAdapterError> {
        let already_delivered = self.cursor.runtime_payloads_delivered;
        let remaining_capacity =
            WEBUI_RUNTIME_ITEM_MAX_PAYLOADS.saturating_sub(self.runtime_payloads_pushed);
        if let Some(runtime_item) = item_to_payloads(
            item,
            scope,
            self.cursor.runtime_item,
            already_delivered,
            remaining_capacity,
        )? {
            return self.push_runtime_payloads(
                runtime_item.final_cursor,
                runtime_item.item_cursor,
                runtime_item.payloads,
                runtime_item.total,
                runtime_item.already_delivered,
            );
        }
        Ok(true)
    }

    fn has_runtime_payload_capacity(&self) -> bool {
        self.runtime_payloads_pushed < WEBUI_RUNTIME_ITEM_MAX_PAYLOADS
    }

    fn push_turn(&mut self, cursor: TurnEventProjectionCursor, payload: ProductOutboundPayload) {
        self.cursor.turn = Some(cursor);
        self.push(payload);
    }

    fn push(&mut self, payload: ProductOutboundPayload) {
        self.payloads.push((self.cursor.clone(), payload));
    }

    fn into_payloads(
        self,
    ) -> impl Iterator<Item = (WebuiProjectionCursor, ProductOutboundPayload)> {
        self.payloads.into_iter()
    }
}

fn runtime_projection_scope(actor: &TurnActor, scope: &TurnScope) -> EventProjectionScope {
    EventProjectionScope {
        stream: EventStreamKey::new(
            scope.tenant_id.clone(),
            actor.user_id.clone(),
            scope.agent_id.clone(),
        ),
        read_scope: ReadScope {
            project_id: scope.project_id.clone(),
            mission_id: None,
            thread_id: Some(scope.thread_id.clone()),
            process_id: None,
        },
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct WebuiProjectionCursor {
    runtime: Option<EventProjectionCursor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runtime_item: Option<EventCursor>,
    turn: Option<TurnEventProjectionCursor>,
    #[serde(default, skip_serializing_if = "is_zero")]
    runtime_payloads_delivered: usize,
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

fn parse_webui_projection_cursor(
    cursor: &str,
) -> Result<WebuiProjectionCursor, ProductAdapterError> {
    if let Ok(parsed) = serde_json::from_str::<WebuiProjectionCursor>(cursor)
        && (parsed.runtime.is_some()
            || parsed.turn.is_some()
            || parsed.runtime_payloads_delivered > 0)
    {
        if parsed.runtime_payloads_delivered > WEBUI_RUNTIME_ITEM_MAX_PAYLOADS + 1 {
            return Err(ProductAdapterError::InvalidIdentifier {
                kind: "projection_cursor",
                reason: "runtime delivery offset exceeds runtime item payload limit".to_string(),
            });
        }
        return Ok(parsed);
    }
    let runtime = serde_json::from_str::<EventProjectionCursor>(cursor).map_err(|_| {
        ProductAdapterError::InvalidIdentifier {
            kind: "projection_cursor",
            reason: "must be a WebUI projection cursor".to_string(),
        }
    })?;
    Ok(WebuiProjectionCursor {
        runtime: Some(runtime),
        runtime_item: None,
        turn: None,
        runtime_payloads_delivered: 0,
    })
}

fn validate_webui_projection_cursor_scope(
    cursor: &WebuiProjectionCursor,
    scope: &TurnScope,
    projection_scope: &EventProjectionScope,
) -> Result<(), ProductAdapterError> {
    if let Some(runtime) = cursor.runtime.as_ref()
        && &runtime.scope != projection_scope
    {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind: "projection_cursor",
            reason: "runtime cursor scope does not match subscription scope".to_string(),
        });
    }
    if let Some(turn) = cursor.turn.as_ref()
        && &turn.scope != scope
    {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind: "projection_cursor",
            reason: "turn cursor scope does not match subscription scope".to_string(),
        });
    }
    Ok(())
}

fn product_cursor_from_webui_cursor(
    cursor: &WebuiProjectionCursor,
) -> Result<ProductProjectionCursor, ProductAdapterError> {
    ProductProjectionCursor::new(
        serde_json::to_string(cursor).map_err(|_| internal_projection_error("cursor encode"))?,
    )
}

fn item_to_payloads(
    item: ProjectionStreamItem,
    scope: &TurnScope,
    expected_item: Option<EventCursor>,
    already_delivered: usize,
    capacity: usize,
) -> RuntimePayloadItemResult {
    match item {
        ProjectionStreamItem::Snapshot(envelope) => {
            let cursor = envelope.cursor();
            snapshot_payloads(
                scope,
                snapshot_from_envelope(envelope)?,
                cursor,
                expected_item,
                already_delivered,
                capacity,
            )
        }
        ProjectionStreamItem::Update(envelope) => {
            let cursor = envelope.cursor();
            replay_payloads(
                scope,
                replay_from_envelope(envelope.as_ref())?,
                cursor,
                expected_item,
                already_delivered,
                capacity,
            )
        }
        ProjectionStreamItem::RebaseRequired { snapshot, .. } => {
            let cursor = snapshot.cursor();
            snapshot_payloads(
                scope,
                snapshot_from_envelope(*snapshot)?,
                cursor,
                expected_item,
                already_delivered,
                capacity,
            )
        }
        ProjectionStreamItem::Lagged { .. } => Err(ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::Unavailable,
            status_code: 503,
            retryable: true,
            reason: RedactedString::new("projection stream lagged; reconnect from origin"),
        }),
        ProjectionStreamItem::KeepAlive => Ok(None),
    }
}

fn snapshot_payloads(
    scope: &TurnScope,
    snapshot: ProjectionSnapshot,
    cursor: EventProjectionCursor,
    expected_item: Option<EventCursor>,
    already_delivered: usize,
    capacity: usize,
) -> RuntimePayloadItemResult {
    let item_cursor = snapshot_item_cursor(&snapshot, &cursor);
    let candidates = snapshot_payload_candidates(snapshot);
    if candidates.is_empty() {
        return Ok(None);
    }
    let total = candidates.len();
    let already_delivered =
        effective_runtime_payload_offset(already_delivered, expected_item, item_cursor.runtime);
    if already_delivered > 0 && already_delivered >= total {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind: "projection_cursor",
            reason: "runtime delivery offset exceeds runtime item payload count".to_string(),
        });
    }
    let payloads = runtime_payloads_from_candidates(
        scope,
        candidates,
        StatePayloadKind::Snapshot,
        already_delivered,
        capacity,
    )?;
    Ok(Some(RuntimePayloadItem {
        final_cursor: cursor,
        item_cursor,
        payloads,
        total,
        already_delivered,
    }))
}

fn replay_payloads(
    scope: &TurnScope,
    replay: &ProjectionReplay,
    cursor: EventProjectionCursor,
    expected_item: Option<EventCursor>,
    already_delivered: usize,
    capacity: usize,
) -> RuntimePayloadItemResult {
    let item_cursor = replay_item_cursor(replay, &cursor);
    let candidates = replay_payload_candidates(replay);
    if candidates.is_empty() {
        return Ok(None);
    }
    let total = candidates.len();
    let already_delivered =
        effective_runtime_payload_offset(already_delivered, expected_item, item_cursor.runtime);
    if already_delivered > 0 && already_delivered >= total {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind: "projection_cursor",
            reason: "runtime delivery offset exceeds runtime item payload count".to_string(),
        });
    }
    let payloads = runtime_payloads_from_candidates(
        scope,
        candidates,
        StatePayloadKind::Update,
        already_delivered,
        capacity,
    )?;
    Ok(Some(RuntimePayloadItem {
        final_cursor: cursor,
        item_cursor,
        payloads,
        total,
        already_delivered,
    }))
}

#[derive(Debug)]
struct RuntimePayloadItem {
    final_cursor: EventProjectionCursor,
    item_cursor: EventProjectionCursor,
    payloads: Vec<ProductOutboundPayload>,
    total: usize,
    already_delivered: usize,
}

type RuntimePayloadItemResult = Result<Option<RuntimePayloadItem>, ProductAdapterError>;

enum RuntimePayloadCandidate {
    State { runs: Vec<RunStatusProjection> },
    CapabilityActivity(CapabilityActivityProjection),
}

#[derive(Clone, Copy)]
enum StatePayloadKind {
    Snapshot,
    Update,
}

fn snapshot_payload_candidates(snapshot: ProjectionSnapshot) -> Vec<RuntimePayloadCandidate> {
    runtime_payload_candidates(
        snapshot.runs,
        snapshot.capability_activities,
        WEBUI_RUNTIME_ITEM_MAX_PAYLOADS,
    )
}

fn replay_payload_candidates(replay: &ProjectionReplay) -> Vec<RuntimePayloadCandidate> {
    runtime_payload_candidates(
        replay.runs.clone(),
        replay.capability_activities.clone(),
        WEBUI_RUNTIME_ITEM_MAX_PAYLOADS,
    )
}

fn runtime_payload_candidates(
    runs: Vec<RunStatusProjection>,
    capability_activities: Vec<CapabilityActivityProjection>,
    max_payloads: usize,
) -> Vec<RuntimePayloadCandidate> {
    let state_payloads = usize::from(!runs.is_empty());
    let activity_payloads = max_payloads.saturating_sub(state_payloads);
    let mut candidates = Vec::with_capacity(
        state_payloads.saturating_add(activity_payloads.min(capability_activities.len())),
    );
    if !runs.is_empty() {
        candidates.push(RuntimePayloadCandidate::State { runs });
    }
    candidates.extend(
        capability_activities
            .into_iter()
            .take(activity_payloads)
            .map(RuntimePayloadCandidate::CapabilityActivity),
    );
    candidates
}

fn runtime_payloads_from_candidates(
    scope: &TurnScope,
    candidates: Vec<RuntimePayloadCandidate>,
    state_kind: StatePayloadKind,
    already_delivered: usize,
    capacity: usize,
) -> Result<Vec<ProductOutboundPayload>, ProductAdapterError> {
    candidates
        .into_iter()
        .skip(already_delivered)
        .take(capacity)
        .map(|candidate| runtime_payload_from_candidate(scope, candidate, state_kind))
        .collect()
}

fn runtime_payload_from_candidate(
    scope: &TurnScope,
    candidate: RuntimePayloadCandidate,
    state_kind: StatePayloadKind,
) -> Result<ProductOutboundPayload, ProductAdapterError> {
    match candidate {
        RuntimePayloadCandidate::State { runs, .. } => {
            let state = run_status_projection_state(scope, runs)?
                .ok_or_else(|| internal_projection_error("missing run projection state"))?;
            let payload = match state_kind {
                StatePayloadKind::Snapshot => ProductOutboundPayload::ProjectionSnapshot { state },
                StatePayloadKind::Update => ProductOutboundPayload::ProjectionUpdate { state },
            };
            Ok(payload)
        }
        RuntimePayloadCandidate::CapabilityActivity(activity) => {
            CapabilityActivityView::new(CapabilityActivityViewInput {
                invocation_id: activity.invocation_id,
                thread_id: activity.thread_id,
                capability_id: activity.capability_id,
                status: capability_activity_status_wire(activity.status),
                provider: activity.provider,
                runtime: activity.runtime,
                process_id: activity.process_id,
                output_bytes: activity.output_bytes,
                error_kind: activity.error_kind,
                updated_at: activity.updated_at,
            })
            .map(ProductOutboundPayload::CapabilityActivity)
        }
    }
}

fn effective_runtime_payload_offset(
    already_delivered: usize,
    expected_item: Option<EventCursor>,
    item_cursor: EventCursor,
) -> usize {
    if already_delivered > 0 && expected_item.is_some() && expected_item != Some(item_cursor) {
        0
    } else {
        already_delivered
    }
}

fn max_projection_cursor(
    left: EventProjectionCursor,
    right: EventProjectionCursor,
) -> EventProjectionCursor {
    if right.runtime > left.runtime {
        right
    } else {
        left
    }
}

fn snapshot_item_cursor(
    snapshot: &ProjectionSnapshot,
    fallback: &EventProjectionCursor,
) -> EventProjectionCursor {
    let runtime = snapshot
        .runs
        .iter()
        .map(|run| run.last_cursor)
        .chain(
            snapshot
                .capability_activities
                .iter()
                .map(|activity| activity.last_cursor),
        )
        .max()
        .unwrap_or(fallback.runtime);
    EventProjectionCursor::for_scope(fallback.scope.clone(), runtime)
}

fn replay_item_cursor(
    replay: &ProjectionReplay,
    fallback: &EventProjectionCursor,
) -> EventProjectionCursor {
    let runtime = replay
        .runs
        .iter()
        .map(|run| run.last_cursor)
        .chain(
            replay
                .capability_activities
                .iter()
                .map(|activity| activity.last_cursor),
        )
        .max()
        .unwrap_or(fallback.runtime);
    EventProjectionCursor::for_scope(fallback.scope.clone(), runtime)
}

fn snapshot_from_envelope(
    envelope: ironclaw_event_streams::ProductProjectionEnvelope,
) -> Result<ProjectionSnapshot, ProductAdapterError> {
    match envelope {
        ironclaw_event_streams::ProductProjectionEnvelope::ThreadSnapshot(snapshot) => Ok(snapshot),
        _ => Err(internal_projection_error(
            "unexpected projection snapshot envelope",
        )),
    }
}

fn replay_from_envelope(
    envelope: &ironclaw_event_streams::ProductProjectionEnvelope,
) -> Result<&ProjectionReplay, ProductAdapterError> {
    match envelope {
        ironclaw_event_streams::ProductProjectionEnvelope::ThreadUpdates(replay) => Ok(replay),
        _ => Err(internal_projection_error(
            "unexpected projection update envelope",
        )),
    }
}

fn run_status_projection_state(
    scope: &TurnScope,
    runs: Vec<RunStatusProjection>,
) -> Result<Option<ProductProjectionState>, ProductAdapterError> {
    let items = runs
        .into_iter()
        .map(|run| ProductProjectionItem::RunStatus {
            run_id: TurnRunId::from_uuid(run.invocation_id.as_uuid()),
            status: run_status_wire(run.status).to_string(),
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        return Ok(None);
    }
    ProductProjectionState::new(scope.thread_id.to_string(), items).map(Some)
}

fn capability_activity_status_wire(
    status: CapabilityActivityStatus,
) -> CapabilityActivityStatusView {
    match status {
        CapabilityActivityStatus::Started => CapabilityActivityStatusView::Started,
        CapabilityActivityStatus::Running => CapabilityActivityStatusView::Running,
        CapabilityActivityStatus::Completed => CapabilityActivityStatusView::Completed,
        CapabilityActivityStatus::Failed => CapabilityActivityStatusView::Failed,
        CapabilityActivityStatus::Killed => CapabilityActivityStatusView::Killed,
    }
}

fn envelope_to_outbound(
    projection_cursor: ProductProjectionCursor,
    payload: ProductOutboundPayload,
    scope: &TurnScope,
    actor: &TurnActor,
    reply_target_binding_ref: &ReplyTargetBindingRef,
) -> Result<ProductOutboundEnvelope, ProductAdapterError> {
    let adapter_id = ProductAdapterId::new(WEBUI_PROJECTION_ADAPTER_ID)?;
    let installation_id = AdapterInstallationId::new(WEBUI_PROJECTION_INSTALLATION_ID)?;
    let target = ProductOutboundTarget::new(
        reply_target_binding_ref.clone(),
        ExternalConversationRef::new(None, scope.thread_id.to_string(), None, None)?,
        Some(ExternalActorRef::new(
            "webui",
            actor.user_id.as_str(),
            None::<String>,
        )?),
    );
    Ok(ProductOutboundEnvelope::new(
        adapter_id,
        installation_id,
        target,
        projection_cursor,
        payload,
    ))
}

fn run_status_wire(status: RunProjectionStatus) -> &'static str {
    match status {
        RunProjectionStatus::Running => "running",
        RunProjectionStatus::Completed => "completed",
        RunProjectionStatus::Cancelled => "cancelled",
        RunProjectionStatus::Failed => "failed",
        RunProjectionStatus::Killed => "killed",
    }
}

fn map_event_stream_error(error: EventProjectionStreamError) -> ProductAdapterError {
    tracing::warn!(
        component = "event_projection_stream",
        operation = "map_stream_error",
        error = %error,
        error_debug = ?error,
        "event projection stream error mapped to product adapter error"
    );
    match error {
        EventProjectionStreamError::InvalidRequest { reason } => {
            ProductAdapterError::InvalidIdentifier {
                kind: "projection_stream_request",
                reason: reason.to_string(),
            }
        }
        EventProjectionStreamError::AccessDenied => ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::Unauthorized,
            status_code: 403,
            retryable: false,
            reason: RedactedString::new("projection stream access denied"),
        },
        EventProjectionStreamError::AdmissionDenied => ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::Unavailable,
            status_code: 429,
            retryable: true,
            reason: RedactedString::new("projection stream admission denied"),
        },
        EventProjectionStreamError::Source => ProductAdapterError::WorkflowTransient {
            reason: RedactedString::new("projection stream source failed"),
        },
        EventProjectionStreamError::Redaction | EventProjectionStreamError::Outbound => {
            ProductAdapterError::Internal {
                detail: RedactedString::new("projection stream validation failed"),
            }
        }
    }
}

fn internal_projection_error(detail: &'static str) -> ProductAdapterError {
    ProductAdapterError::Internal {
        detail: RedactedString::new(detail),
    }
}

#[cfg(test)]
mod tests;
