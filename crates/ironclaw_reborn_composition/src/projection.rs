use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

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
use ironclaw_host_api::{CapabilityId, InvocationId};
use ironclaw_outbound::InMemoryOutboundStateStore;
use ironclaw_product_adapters::{
    AdapterInstallationId, CapabilityActivityStatusView, CapabilityActivityView,
    CapabilityActivityViewInput, CapabilityDisplayPreviewView, CapabilityDisplayPreviewViewInput,
    ExternalActorRef, ExternalConversationRef, ProductAdapterError, ProductAdapterId,
    ProductOutboundEnvelope, ProductOutboundPayload, ProductOutboundTarget, ProductProjectionItem,
    ProductProjectionState, ProductWorkflowRejectionKind,
    ProjectionCursor as ProductProjectionCursor, ProjectionStream, ProjectionSubscriptionRequest,
    RedactedString,
};
use ironclaw_turns::{
    ReplyTargetBindingRef, TurnActor, TurnCoordinator, TurnEventProjectionCursor,
    TurnEventProjectionSource, TurnRunId, TurnScope, run_profile::CapabilityInputRef,
};

mod turn_events;
use turn_events::{TurnEventBridge, TurnEventPayload};

const WEBUI_PROJECTION_PAGE_LIMIT: usize = 256;
const WEBUI_RUNTIME_ITEM_MAX_PAYLOADS: usize = WEBUI_PROJECTION_PAGE_LIMIT + 1;
const WEBUI_PROJECTION_ADAPTER_ID: &str = "webui_v2";
const WEBUI_PROJECTION_INSTALLATION_ID: &str = "webui_v2.local";
const SANITIZE_JSON_MAX_DEPTH: usize = 32;

#[derive(Clone)]
pub(crate) struct RebornProjectionServices {
    event_stream_manager: Arc<EventStreamManager>,
    turn_events: TurnEventBridge,
    display_previews: Arc<dyn CapabilityDisplayPreviewSource>,
    webui_reply_target_binding_ref: ReplyTargetBindingRef,
}

#[async_trait]
trait CapabilityDisplayPreviewSource: Send + Sync {
    async fn preview(
        &self,
        activity: &CapabilityActivityProjection,
    ) -> Result<Option<CapabilityDisplayPreviewView>, ProductAdapterError>;
}

struct NoopCapabilityDisplayPreviewSource;

#[async_trait]
impl CapabilityDisplayPreviewSource for NoopCapabilityDisplayPreviewSource {
    async fn preview(
        &self,
        _activity: &CapabilityActivityProjection,
    ) -> Result<Option<CapabilityDisplayPreviewView>, ProductAdapterError> {
        Ok(None)
    }
}

#[derive(Default)]
pub(crate) struct CapabilityDisplayPreviewStore {
    pending: Mutex<CapabilityDisplayPendingInputs>,
    completed: Mutex<CapabilityDisplayCompletedPreviews>,
}

#[derive(Default)]
struct CapabilityDisplayPendingInputs {
    by_ref: HashMap<String, CapabilityDisplayInputPreview>,
    refs_by_run: HashMap<String, Vec<String>>,
}

#[derive(Default)]
struct CapabilityDisplayCompletedPreviews {
    by_invocation: HashMap<String, CapabilityDisplayPreviewRecord>,
    invocations_by_run: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone)]
struct CapabilityDisplayInputPreview {
    title: String,
    subtitle: Option<String>,
    input_summary: Option<String>,
    truncated: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct CapabilityDisplayPreviewRecord {
    pub(crate) title: String,
    pub(crate) subtitle: Option<String>,
    pub(crate) input_summary: Option<String>,
    pub(crate) output_summary: Option<String>,
    pub(crate) output_preview: Option<String>,
    pub(crate) output_kind: Option<String>,
    pub(crate) output_bytes: Option<u64>,
    pub(crate) result_ref: Option<String>,
    pub(crate) truncated: bool,
}

pub(crate) struct CapabilityDisplayPreviewResult<'a> {
    pub(crate) run_id: &'a str,
    pub(crate) input_ref: &'a CapabilityInputRef,
    pub(crate) invocation_id: InvocationId,
    pub(crate) capability_id: &'a CapabilityId,
    pub(crate) result_ref: &'a str,
    pub(crate) output: &'a serde_json::Value,
    pub(crate) output_bytes: u64,
}

impl CapabilityDisplayPreviewStore {
    pub(crate) fn record_input(
        &self,
        run_id: &str,
        input_ref: &CapabilityInputRef,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) {
        let input_summary = input_summary(tool_name, arguments);
        let input = CapabilityDisplayInputPreview {
            title: bounded_display_text(tool_name, 2048).text,
            subtitle: safe_path_subtitle(arguments),
            truncated: input_summary
                .as_ref()
                .is_some_and(|summary| summary.truncated),
            input_summary: input_summary.map(|summary| summary.text),
        };
        if let Ok(mut pending) = self.pending.lock() {
            let input_ref = input_ref.as_str().to_string();
            pending.by_ref.insert(input_ref.clone(), input);
            pending
                .refs_by_run
                .entry(run_id.to_string())
                .or_default()
                .push(input_ref);
        }
    }

    pub(crate) fn record_result(&self, result: CapabilityDisplayPreviewResult<'_>) {
        let input = self
            .pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.by_ref.remove(result.input_ref.as_str()));
        let title = input
            .as_ref()
            .map(|input| input.title.clone())
            .unwrap_or_else(|| safe_capability_title(result.capability_id.as_str()).to_string());
        let output = output_preview(result.output);
        let record = CapabilityDisplayPreviewRecord {
            title,
            subtitle: input.as_ref().and_then(|input| input.subtitle.clone()),
            input_summary: input.as_ref().and_then(|input| input.input_summary.clone()),
            output_summary: output.summary,
            output_preview: output.preview,
            output_kind: Some(output.kind),
            output_bytes: Some(result.output_bytes),
            result_ref: Some(result.result_ref.to_string()),
            truncated: input.as_ref().is_some_and(|input| input.truncated) || output.truncated,
        };
        if let Ok(mut completed) = self.completed.lock() {
            let invocation_id = result.invocation_id.to_string();
            completed
                .by_invocation
                .insert(invocation_id.clone(), record);
            completed
                .invocations_by_run
                .entry(result.run_id.to_string())
                .or_default()
                .push(invocation_id);
        }
    }

    pub(crate) fn prune_run(&self, run_id: &str) {
        if let Ok(mut pending) = self.pending.lock()
            && let Some(input_refs) = pending.refs_by_run.remove(run_id)
        {
            for input_ref in input_refs {
                pending.by_ref.remove(&input_ref);
            }
        }
        if let Ok(mut completed) = self.completed.lock()
            && let Some(invocation_ids) = completed.invocations_by_run.remove(run_id)
        {
            for invocation_id in invocation_ids {
                completed.by_invocation.remove(&invocation_id);
            }
        }
    }

    pub(crate) fn record_for_invocation(
        &self,
        invocation_id: InvocationId,
    ) -> Option<CapabilityDisplayPreviewRecord> {
        self.completed.lock().ok().and_then(|completed| {
            completed
                .by_invocation
                .get(&invocation_id.to_string())
                .cloned()
        })
    }
}

#[async_trait]
impl CapabilityDisplayPreviewSource for CapabilityDisplayPreviewStore {
    async fn preview(
        &self,
        activity: &CapabilityActivityProjection,
    ) -> Result<Option<CapabilityDisplayPreviewView>, ProductAdapterError> {
        capability_display_preview_from_store(self, activity)
    }
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

    pub(crate) fn with_display_previews(
        mut self,
        display_previews: Arc<CapabilityDisplayPreviewStore>,
    ) -> Self {
        self.display_previews = display_previews;
        self
    }

    pub(crate) fn webui_event_stream(&self) -> Arc<dyn ProjectionStream> {
        Arc::new(WebuiRuntimeProjectionStream {
            manager: Arc::clone(&self.event_stream_manager),
            turn_events: self.turn_events.clone(),
            display_previews: Arc::clone(&self.display_previews),
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
        display_previews: Arc::new(NoopCapabilityDisplayPreviewSource),
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
    display_previews: Arc<dyn CapabilityDisplayPreviewSource>,
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
            && batch
                .push_runtime_item(item, &request.scope, self.display_previews.as_ref())
                .await?
            && !resumes_runtime_item
        {
            for _ in 1..WEBUI_PROJECTION_PAGE_LIMIT {
                if !batch.has_runtime_payload_capacity() {
                    break;
                }
                let Some(item) = subscription.try_next_buffered() else {
                    break;
                };
                if !batch
                    .push_runtime_item(item, &request.scope, self.display_previews.as_ref())
                    .await?
                {
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
                if !batch
                    .push_runtime_item(item, &request.scope, self.display_previews.as_ref())
                    .await?
                {
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

    async fn push_runtime_item(
        &mut self,
        item: ProjectionStreamItem,
        scope: &TurnScope,
        display_previews: &dyn CapabilityDisplayPreviewSource,
    ) -> Result<bool, ProductAdapterError> {
        let already_delivered = self.cursor.runtime_payloads_delivered;
        let remaining_capacity =
            WEBUI_RUNTIME_ITEM_MAX_PAYLOADS.saturating_sub(self.runtime_payloads_pushed);
        if let Some(runtime_item) = item_to_payloads(
            item,
            scope,
            display_previews,
            self.cursor.runtime_item,
            already_delivered,
            remaining_capacity,
        )
        .await?
        {
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

async fn item_to_payloads(
    item: ProjectionStreamItem,
    scope: &TurnScope,
    display_previews: &dyn CapabilityDisplayPreviewSource,
    expected_item: Option<EventCursor>,
    already_delivered: usize,
    capacity: usize,
) -> RuntimePayloadItemResult {
    match item {
        ProjectionStreamItem::Snapshot(envelope) => {
            let cursor = envelope.cursor();
            snapshot_payloads(
                scope,
                display_previews,
                snapshot_from_envelope(envelope)?,
                cursor,
                expected_item,
                already_delivered,
                capacity,
            )
            .await
        }
        ProjectionStreamItem::Update(envelope) => {
            let cursor = envelope.cursor();
            replay_payloads(
                scope,
                display_previews,
                replay_from_envelope(envelope.as_ref())?,
                cursor,
                expected_item,
                already_delivered,
                capacity,
            )
            .await
        }
        ProjectionStreamItem::RebaseRequired { snapshot, .. } => {
            let cursor = snapshot.cursor();
            snapshot_payloads(
                scope,
                display_previews,
                snapshot_from_envelope(*snapshot)?,
                cursor,
                expected_item,
                already_delivered,
                capacity,
            )
            .await
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

async fn snapshot_payloads(
    scope: &TurnScope,
    display_previews: &dyn CapabilityDisplayPreviewSource,
    snapshot: ProjectionSnapshot,
    cursor: EventProjectionCursor,
    expected_item: Option<EventCursor>,
    already_delivered: usize,
    capacity: usize,
) -> RuntimePayloadItemResult {
    let item_cursor = snapshot_item_cursor(&snapshot, &cursor);
    let candidates = snapshot_payload_candidates(snapshot);
    let all_payloads = runtime_payloads_from_candidates(
        scope,
        display_previews,
        candidates,
        StatePayloadKind::Snapshot,
    )
    .await?;
    if all_payloads.is_empty() {
        return Ok(None);
    }
    let total = all_payloads.len();
    let already_delivered =
        effective_runtime_payload_offset(already_delivered, expected_item, item_cursor.runtime);
    if already_delivered > 0 && already_delivered >= total {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind: "projection_cursor",
            reason: "runtime delivery offset exceeds runtime item payload count".to_string(),
        });
    }
    let payloads = all_payloads
        .into_iter()
        .skip(already_delivered)
        .take(capacity)
        .collect();
    Ok(Some(RuntimePayloadItem {
        final_cursor: cursor,
        item_cursor,
        payloads,
        total,
        already_delivered,
    }))
}

async fn replay_payloads(
    scope: &TurnScope,
    display_previews: &dyn CapabilityDisplayPreviewSource,
    replay: &ProjectionReplay,
    cursor: EventProjectionCursor,
    expected_item: Option<EventCursor>,
    already_delivered: usize,
    capacity: usize,
) -> RuntimePayloadItemResult {
    let item_cursor = replay_item_cursor(replay, &cursor);
    let candidates = replay_payload_candidates(replay);
    let all_payloads = runtime_payloads_from_candidates(
        scope,
        display_previews,
        candidates,
        StatePayloadKind::Update,
    )
    .await?;
    if all_payloads.is_empty() {
        return Ok(None);
    }
    let total = all_payloads.len();
    let already_delivered =
        effective_runtime_payload_offset(already_delivered, expected_item, item_cursor.runtime);
    if already_delivered > 0 && already_delivered >= total {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind: "projection_cursor",
            reason: "runtime delivery offset exceeds runtime item payload count".to_string(),
        });
    }
    let payloads = all_payloads
        .into_iter()
        .skip(already_delivered)
        .take(capacity)
        .collect();
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
    CapabilityDisplayPreview(CapabilityActivityProjection),
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
    for activity in capability_activities.into_iter().take(activity_payloads) {
        candidates.push(RuntimePayloadCandidate::CapabilityActivity(
            activity.clone(),
        ));
        candidates.push(RuntimePayloadCandidate::CapabilityDisplayPreview(activity));
    }
    candidates
}

async fn runtime_payloads_from_candidates(
    scope: &TurnScope,
    display_previews: &dyn CapabilityDisplayPreviewSource,
    candidates: Vec<RuntimePayloadCandidate>,
    state_kind: StatePayloadKind,
) -> Result<Vec<ProductOutboundPayload>, ProductAdapterError> {
    let mut payloads = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if let Some(payload) =
            runtime_payload_from_candidate(scope, display_previews, candidate, state_kind).await?
        {
            payloads.push(payload);
        }
    }
    Ok(payloads)
}

async fn runtime_payload_from_candidate(
    scope: &TurnScope,
    display_previews: &dyn CapabilityDisplayPreviewSource,
    candidate: RuntimePayloadCandidate,
    state_kind: StatePayloadKind,
) -> Result<Option<ProductOutboundPayload>, ProductAdapterError> {
    match candidate {
        RuntimePayloadCandidate::State { runs, .. } => {
            let state = run_status_projection_state(scope, runs)?
                .ok_or_else(|| internal_projection_error("missing run projection state"))?;
            let payload = match state_kind {
                StatePayloadKind::Snapshot => ProductOutboundPayload::ProjectionSnapshot { state },
                StatePayloadKind::Update => ProductOutboundPayload::ProjectionUpdate { state },
            };
            Ok(Some(payload))
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
            .map(Some)
        }
        RuntimePayloadCandidate::CapabilityDisplayPreview(activity) => display_previews
            .preview(&activity)
            .await
            .map(|preview| preview.map(ProductOutboundPayload::CapabilityDisplayPreview)),
    }
}

fn capability_display_preview_from_store(
    store: &CapabilityDisplayPreviewStore,
    activity: &CapabilityActivityProjection,
) -> Result<Option<CapabilityDisplayPreviewView>, ProductAdapterError> {
    if !matches!(
        activity.status,
        CapabilityActivityStatus::Completed
            | CapabilityActivityStatus::Failed
            | CapabilityActivityStatus::Killed
    ) {
        return Ok(None);
    }
    let Some(record) = store.record_for_invocation(activity.invocation_id) else {
        return failed_capability_display_preview(activity);
    };
    CapabilityDisplayPreviewView::new(CapabilityDisplayPreviewViewInput {
        invocation_id: activity.invocation_id,
        thread_id: activity.thread_id.clone(),
        capability_id: activity.capability_id.clone(),
        status: capability_activity_status_wire(activity.status),
        title: record.title,
        subtitle: record.subtitle,
        input_summary: record.input_summary,
        output_summary: record.output_summary,
        output_preview: record.output_preview,
        output_kind: record.output_kind,
        output_bytes: activity.output_bytes.or(record.output_bytes),
        result_ref: record.result_ref,
        truncated: record.truncated,
        updated_at: activity.updated_at,
    })
    .map(Some)
}

fn failed_capability_display_preview(
    activity: &CapabilityActivityProjection,
) -> Result<Option<CapabilityDisplayPreviewView>, ProductAdapterError> {
    if !matches!(
        activity.status,
        CapabilityActivityStatus::Failed | CapabilityActivityStatus::Killed
    ) {
        return Ok(None);
    }
    let summary = activity
        .error_kind
        .as_deref()
        .map(|kind| format!("tool failed: {}", sanitize_text(kind)))
        .unwrap_or_else(|| "tool failed".to_string());
    CapabilityDisplayPreviewView::new(CapabilityDisplayPreviewViewInput {
        invocation_id: activity.invocation_id,
        thread_id: activity.thread_id.clone(),
        capability_id: activity.capability_id.clone(),
        status: capability_activity_status_wire(activity.status),
        title: bounded_display_text(safe_capability_title(activity.capability_id.as_str()), 2048)
            .text,
        subtitle: None,
        input_summary: None,
        output_summary: Some(summary.clone()),
        output_preview: Some(summary),
        output_kind: Some("text".to_string()),
        output_bytes: activity.output_bytes,
        result_ref: None,
        truncated: false,
        updated_at: activity.updated_at,
    })
    .map(Some)
}

#[derive(Debug, Clone)]
struct OutputPreview {
    summary: Option<String>,
    preview: Option<String>,
    kind: String,
    truncated: bool,
}

fn output_preview(value: &serde_json::Value) -> OutputPreview {
    let (kind, text, json_truncated) = if let Some(text) = value.as_str() {
        ("text", text.to_string(), false)
    } else if let Some(text) = value
        .get("content")
        .or_else(|| value.get("text"))
        .or_else(|| value.get("stdout"))
        .and_then(serde_json::Value::as_str)
    {
        ("text", text.to_string(), false)
    } else {
        let safe_value = sanitize_json_value_with_truncation(value);
        (
            "json",
            serde_json::to_string_pretty(&safe_value.value).unwrap_or_else(|_| "{}".to_string()),
            safe_value.truncated,
        )
    };
    let preview = bounded_preview_text(&text);
    let summary = bounded_display_text(
        match kind {
            "text" => "text output",
            _ => "json output",
        },
        2048,
    );
    OutputPreview {
        summary: non_empty(summary.text),
        preview: non_empty(preview.text),
        kind: kind.to_string(),
        truncated: summary.truncated || preview.truncated || json_truncated,
    }
}

#[derive(Debug, Clone)]
struct DisplayText {
    text: String,
    truncated: bool,
}

fn input_summary(capability_id: &str, value: &serde_json::Value) -> Option<DisplayText> {
    if (capability_id == "read_file"
        || capability_id == "builtin.read_file"
        || capability_id.ends_with(".read_file"))
        && let Some(path) = safe_path_subtitle(value)
    {
        let mut summary = format!("path: {path}");
        if let Some(max_bytes) = value.get("max_bytes").and_then(serde_json::Value::as_u64) {
            summary.push_str(&format!("\nmax_bytes: {max_bytes}"));
        }
        return Some(bounded_display_text(&summary, 2048));
    }
    let safe_value = sanitize_json_value_with_truncation(value);
    serde_json::to_string_pretty(&safe_value.value)
        .ok()
        .map(|text| {
            let mut summary = bounded_display_text(&text, 2048);
            summary.truncated |= safe_value.truncated;
            summary
        })
}

fn safe_capability_title(capability_id: &str) -> &str {
    capability_id
        .rsplit_once('.')
        .map(|(_, suffix)| suffix)
        .unwrap_or(capability_id)
}

fn safe_path_subtitle(value: &serde_json::Value) -> Option<String> {
    let path = value
        .get("path")
        .or_else(|| value.get("file_path"))
        .or_else(|| value.get("target"))?
        .as_str()?;
    safe_display_path(path)
}

fn safe_display_path(path: &str) -> Option<String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.starts_with('~')
        || path.contains("..")
        || path.contains('\\')
        || path.chars().any(char::is_control)
    {
        return None;
    }
    Some(bounded_display_text(path, 2048).text)
}

#[derive(Debug, Clone)]
struct SanitizedJson {
    value: serde_json::Value,
    truncated: bool,
}

#[cfg(test)]
fn sanitize_json_value(value: &serde_json::Value) -> serde_json::Value {
    sanitize_json_value_with_truncation(value).value
}

fn sanitize_json_value_with_truncation(value: &serde_json::Value) -> SanitizedJson {
    sanitize_json_value_at_depth(value, SANITIZE_JSON_MAX_DEPTH)
}

fn sanitize_json_value_at_depth(
    value: &serde_json::Value,
    remaining_depth: usize,
) -> SanitizedJson {
    if remaining_depth == 0 {
        return SanitizedJson {
            value: serde_json::Value::String("[truncated]".to_string()),
            truncated: true,
        };
    }
    match value {
        serde_json::Value::Object(map) => {
            let mut truncated = false;
            let value = serde_json::Value::Object(
                map.iter()
                    .map(|(key, value)| {
                        let sanitized = if is_sensitive_key(key) {
                            serde_json::Value::String("[redacted]".to_string())
                        } else {
                            let sanitized =
                                sanitize_json_value_at_depth(value, remaining_depth - 1);
                            truncated |= sanitized.truncated;
                            sanitized.value
                        };
                        (key.clone(), sanitized)
                    })
                    .collect(),
            );
            SanitizedJson { value, truncated }
        }
        serde_json::Value::Array(values) => {
            let mut truncated = false;
            let value = serde_json::Value::Array(
                values
                    .iter()
                    .map(|value| {
                        let sanitized = sanitize_json_value_at_depth(value, remaining_depth - 1);
                        truncated |= sanitized.truncated;
                        sanitized.value
                    })
                    .collect(),
            );
            SanitizedJson { value, truncated }
        }
        serde_json::Value::String(value) => SanitizedJson {
            value: serde_json::Value::String(sanitize_text(value)),
            truncated: false,
        },
        other => SanitizedJson {
            value: other.clone(),
            truncated: false,
        },
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    // Display previews bias toward over-redaction; benign counters like max_tokens can be hidden.
    key.contains("secret")
        || key.contains("password")
        || key.contains("token")
        || key.contains("credential")
        || key.contains("api_key")
        || key == "key"
}

fn bounded_display_text(text: &str, max_bytes: usize) -> DisplayText {
    let sanitized = sanitize_text(text);
    truncate_bytes(&sanitized, max_bytes)
}

fn bounded_preview_text(text: &str) -> DisplayText {
    let mut sanitized = sanitize_text(text);
    let mut truncated = false;
    let mut line_count = 0usize;
    let mut end = sanitized.len();
    // Keep in sync with ironclaw_product_adapters display-preview validator limits.
    for (index, _) in sanitized.match_indices('\n') {
        line_count += 1;
        if line_count >= 120 {
            end = index;
            truncated = true;
            break;
        }
    }
    if truncated {
        sanitized.truncate(end);
    }
    let mut bounded = truncate_bytes(&sanitized, 16 * 1024);
    bounded.truncated |= truncated;
    bounded
}

fn non_empty(text: String) -> Option<String> {
    if text.is_empty() { None } else { Some(text) }
}

fn truncate_bytes(text: &str, max_bytes: usize) -> DisplayText {
    if text.len() <= max_bytes {
        return DisplayText {
            text: text.to_string(),
            truncated: false,
        };
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    DisplayText {
        text: text[..end].to_string(),
        truncated: true,
    }
}

fn sanitize_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for token in text.split_inclusive(char::is_whitespace) {
        let trimmed = token.trim_end();
        let suffix = &token[trimmed.len()..];
        if is_secret_like(trimmed) || is_unsafe_path_like(trimmed) {
            out.push_str("[redacted]");
            push_safe_text(&mut out, suffix);
        } else {
            push_safe_text(&mut out, token);
        }
    }
    out
}

fn push_safe_text(out: &mut String, text: &str) {
    out.extend(
        text.chars().filter(|character| {
            *character == '\n' || *character == '\t' || !character.is_control()
        }),
    );
}

fn is_secret_like(token: &str) -> bool {
    let lower = token
        .trim_matches(token_boundary_punctuation)
        .to_ascii_lowercase();
    lower.starts_with("sk-")
        || lower.contains("api_key=")
        || lower.contains("api_key:")
        || lower.contains("access_token=")
        || lower.contains("access_token:")
        || lower.contains("secret=")
        || lower.contains("secret:")
        || lower.contains("password=")
        || lower.contains("password:")
        || lower.contains("token=")
        || lower.contains("token:")
}

fn is_unsafe_path_like(token: &str) -> bool {
    let token = token.trim_matches(token_boundary_punctuation);
    token_contains_absolute_posix_path(token)
        || token.starts_with("\\\\")
        || token.contains("\\\\")
        || token.get(1..3) == Some(":\\")
}

fn token_contains_absolute_posix_path(token: &str) -> bool {
    let mut previous = None;
    let mut characters = token.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '/'
            && previous.is_none_or(token_boundary_punctuation)
            && !matches!(previous, Some('/'))
            && !matches!(characters.peek(), Some('/'))
        {
            return true;
        }
        previous = Some(character);
    }
    false
}

fn token_boundary_punctuation(character: char) -> bool {
    matches!(
        character,
        '"' | '\'' | '`' | ',' | ';' | ':' | '=' | '(' | ')' | '[' | ']' | '{' | '}'
    )
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
