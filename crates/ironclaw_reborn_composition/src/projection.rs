use std::sync::Arc;

use async_trait::async_trait;
use futures::{StreamExt, stream};
#[cfg(test)]
use ironclaw_event_projections::CapabilityActivityProjection;
use ironclaw_event_projections::{
    CapabilityActivityStatus, EventProjectionService, ProjectionCursor as EventProjectionCursor,
    ProjectionReplay, ProjectionScope as EventProjectionScope, ProjectionSnapshot,
    ReplayEventProjectionService, RunProjectionStatus, RunStatusProjection,
};
use ironclaw_event_streams::{
    AllowAllProjectionAccessPolicy, EventStreamManager, InMemoryProjectionStreamAdmissionPolicy,
    InMemoryProjectionUpdateSource, NoExposureProjectionRedactionValidator,
    ProjectionStreamError as EventProjectionStreamError, ProjectionStreamItem,
    ProjectionSubscribeRequest, ProjectionSubscription as EventProjectionSubscription,
    ProjectionTarget, ProjectionViewClass, SubscriberCapabilities, ThreadLiveProjectionUpdate,
};
use ironclaw_events::{DurableEventLog, EventCursor, EventStreamKey, ReadScope};
use ironclaw_first_party_extension_ports::SkillActivationObserver;
use ironclaw_host_api::UserId;
use ironclaw_outbound::InMemoryOutboundStateStore;
use ironclaw_product_adapters::{
    AdapterInstallationId, CapabilityActivityStatusView, CapabilityActivityView,
    CapabilityActivityViewInput, ExternalActorRef, ExternalConversationRef, ProductAdapterError,
    ProductAdapterId, ProductOutboundEnvelope, ProductOutboundPayload, ProductOutboundTarget,
    ProductProjectionItem, ProductProjectionState, ProductWorkflowRejectionKind,
    ProjectionCursor as ProductProjectionCursor, ProjectionStream, ProjectionSubscriptionRequest,
    RedactedString,
};
use ironclaw_run_state::ApprovalRequestStore;
use ironclaw_turns::{
    ReplyTargetBindingRef, SanitizedFailure, TurnActor, TurnCoordinator, TurnEventProjectionCursor,
    TurnEventProjectionSource, TurnRunId, TurnScope, run_profile::LoopHostMilestoneSink,
};

mod display_preview;
mod live_progress;
mod runtime_replay;
mod turn_events;
use crate::AuthChallengeProvider;
use display_preview::{
    CapabilityDisplayPreviewResolution, CapabilityDisplayPreviewSource,
    NoopCapabilityDisplayPreviewSource,
};
use live_progress::{
    LiveProgressMilestoneSink, LiveProjectionPublisher, LiveSkillActivationObserver,
    product_items_for_live_update,
};
use runtime_replay::{
    DeliveredRuntimePayload, RuntimePayloadCandidate, RuntimePayloadResolution, RuntimePayloads,
    replay_payload_candidates, snapshot_payload_candidates,
};
// Only the Slack delivery path (feature-gated) consumes this re-export.
#[cfg(feature = "slack-v2-host-beta")]
pub(crate) use turn_events::approval_prompt_context_view;
use turn_events::{
    FailureExplanationProvider, ModelFailureExplanationProvider, TurnEventBridge, TurnEventPayload,
};

pub(crate) use display_preview::{CapabilityDisplayPreviewResult, CapabilityDisplayPreviewStore};
#[cfg(test)]
pub(crate) use display_preview::{SANITIZE_JSON_MAX_DEPTH, sanitize_json_value, sanitize_text};

const WEBUI_PROJECTION_PAGE_LIMIT: usize = 256;
const WEBUI_RUNTIME_ITEM_MAX_PAYLOADS: usize = WEBUI_PROJECTION_PAGE_LIMIT + 1;
const WEBUI_PROJECTION_ADAPTER_ID: &str = "webui_v2";
const WEBUI_PROJECTION_INSTALLATION_ID: &str = "webui_v2.local";

#[derive(Clone)]
pub(crate) struct RebornProjectionServices {
    event_stream_manager: Arc<EventStreamManager>,
    live_updates: Arc<InMemoryProjectionUpdateSource>,
    turn_events: TurnEventBridge,
    approval_requests: Option<Arc<dyn ApprovalRequestStore>>,
    display_previews: Arc<dyn CapabilityDisplayPreviewSource>,
    webui_reply_target_binding_ref: ReplyTargetBindingRef,
    auth_challenges: Option<Arc<dyn AuthChallengeProvider>>,
}

impl RebornProjectionServices {
    pub(crate) fn with_turn_events(
        mut self,
        turn_event_source: Arc<dyn TurnEventProjectionSource>,
        turn_coordinator: Arc<dyn TurnCoordinator>,
    ) -> Self {
        self.turn_events = TurnEventBridge::enabled(
            turn_event_source,
            turn_coordinator,
            self.approval_requests.clone(),
        );
        self
    }

    pub(crate) fn with_approval_requests(
        mut self,
        approval_requests: Arc<dyn ApprovalRequestStore>,
    ) -> Self {
        self.approval_requests = Some(approval_requests.clone());
        self.turn_events = self
            .turn_events
            .with_approval_requests(Some(approval_requests));
        self
    }

    pub(crate) fn with_failure_explainer(
        mut self,
        explainer: Arc<dyn FailureExplanationProvider>,
    ) -> Self {
        self.turn_events = self.turn_events.with_failure_explainer(explainer);
        self
    }

    pub(crate) fn with_model_failure_explainer_factory(
        self,
        system_inference: Arc<
            dyn Fn() -> Arc<dyn ironclaw_turns::run_profile::SystemInferencePort> + Send + Sync,
        >,
    ) -> Self {
        self.with_failure_explainer(Arc::new(ModelFailureExplanationProvider::from_factory(
            system_inference,
        )))
    }

    /// Wire in an auth challenge provider so `auth_required` SSE events carry
    /// `challenge_kind`, `provider`, `account_label`, and `authorization_url`.
    /// Optional: when absent the `AuthPromptView` omits those fields (backward
    /// compatible — legacy consumers deserialise them as `None`).
    pub(crate) fn with_auth_challenges(mut self, provider: Arc<dyn AuthChallengeProvider>) -> Self {
        self.auth_challenges = Some(provider);
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
            auth_challenges: self.auth_challenges.clone(),
            display_previews: Arc::clone(&self.display_previews),
            reply_target_binding_ref: self.webui_reply_target_binding_ref.clone(),
        })
    }

    pub(crate) fn with_live_progress_milestone_sink_for_publisher(
        &self,
        inner: Arc<dyn LoopHostMilestoneSink>,
        publisher: Arc<LiveProjectionPublisher>,
    ) -> Arc<dyn LoopHostMilestoneSink> {
        Arc::new(LiveProgressMilestoneSink::new(inner, publisher))
    }

    pub(crate) fn live_projection_publisher(
        &self,
        actor_user_id: UserId,
    ) -> Arc<LiveProjectionPublisher> {
        Arc::new(LiveProjectionPublisher::new(
            Arc::clone(&self.live_updates),
            actor_user_id,
        ))
    }

    pub(crate) fn skill_activation_observer(
        &self,
        publisher: Arc<LiveProjectionPublisher>,
    ) -> Arc<dyn SkillActivationObserver> {
        Arc::new(LiveSkillActivationObserver::new(publisher))
    }
}

pub(crate) fn build_reborn_projection_services(
    event_log: Arc<dyn DurableEventLog>,
    webui_reply_target_binding_ref: ReplyTargetBindingRef,
) -> RebornProjectionServices {
    let projection: Arc<dyn EventProjectionService> =
        Arc::new(ReplayEventProjectionService::from_runtime_log(event_log));
    let live_updates = Arc::new(InMemoryProjectionUpdateSource::new(128));
    let event_stream_manager = Arc::new(EventStreamManager::from_services(
        projection,
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        live_updates.clone(),
        Arc::new(NoExposureProjectionRedactionValidator),
        Arc::new(InMemoryOutboundStateStore::default()),
    ));
    RebornProjectionServices {
        event_stream_manager,
        live_updates,
        turn_events: TurnEventBridge::default(),
        approval_requests: None,
        display_previews: Arc::new(NoopCapabilityDisplayPreviewSource),
        webui_reply_target_binding_ref,
        auth_challenges: None,
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
    auth_challenges: Option<Arc<dyn AuthChallengeProvider>>,
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

        let is_resuming_runtime_payloads = origin_cursor.runtime_payloads_delivered > 0;
        let mut batch = WebuiProjectionBatch::new(origin_cursor);
        if let Some(item) = subscription.next().await
            && batch
                .push_runtime_item(item, &request.scope, self.display_previews.as_ref())
                .await?
            && !is_resuming_runtime_payloads
        {
            consume_buffered_runtime_items(
                &mut subscription,
                &mut batch,
                &request.scope,
                self.display_previews.as_ref(),
            )
            .await?;
        }

        if batch.runtime_payloads_pushed == 0 && !is_resuming_runtime_payloads {
            consume_buffered_runtime_items(
                &mut subscription,
                &mut batch,
                &request.scope,
                self.display_previews.as_ref(),
            )
            .await?;
        }

        let turn_after = batch.cursor().turn.clone();
        let turn_drain = self
            .turn_events
            .drain(
                &request.actor.user_id,
                &request.scope,
                turn_after,
                self.auth_challenges.as_deref(),
            )
            .await?;
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

async fn consume_buffered_runtime_items(
    subscription: &mut EventProjectionSubscription,
    batch: &mut WebuiProjectionBatch,
    scope: &TurnScope,
    display_previews: &dyn CapabilityDisplayPreviewSource,
) -> Result<(), ProductAdapterError> {
    for _ in 0..WEBUI_PROJECTION_PAGE_LIMIT {
        if !batch.has_runtime_payload_capacity() {
            break;
        }
        let Some(item) = subscription.try_next_buffered() else {
            break;
        };
        if !batch
            .push_runtime_item(item, scope, display_previews)
            .await?
        {
            break;
        }
    }
    Ok(())
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

    fn push_durable_runtime_payloads(
        &mut self,
        final_cursor: EventProjectionCursor,
        item_cursor: EventProjectionCursor,
        payloads: Vec<DeliveredRuntimePayload>,
        total: usize,
        already_delivered: usize,
    ) -> Result<bool, ProductAdapterError> {
        if total == 0 {
            return Ok(true);
        }

        if already_delivered > total {
            return Err(ProductAdapterError::InvalidIdentifier {
                kind: "projection_cursor",
                reason: "runtime delivery offset exceeds runtime item payload count".to_string(),
            });
        }
        if already_delivered > 0 && already_delivered == total {
            self.cursor.runtime = Some(max_projection_cursor(final_cursor, item_cursor));
            self.cursor.runtime_item = None;
            self.cursor.runtime_payloads_delivered = 0;
            self.push(ProductOutboundPayload::KeepAlive);
            return Ok(true);
        }

        let remaining_capacity =
            WEBUI_RUNTIME_ITEM_MAX_PAYLOADS.saturating_sub(self.runtime_payloads_pushed);
        if remaining_capacity == 0 {
            return Ok(false);
        }

        if payloads.is_empty() {
            return Ok(false);
        }

        for DeliveredRuntimePayload { delivered, payload } in
            payloads.into_iter().take(remaining_capacity)
        {
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

    fn push_live_payload(
        &mut self,
        cursor: EventProjectionCursor,
        payload: ProductOutboundPayload,
    ) -> bool {
        if !self.has_runtime_payload_capacity() {
            return false;
        }
        self.runtime_payloads_pushed += 1;
        self.cursor.live = Some(cursor);
        self.push(payload);
        true
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
            self.cursor.live.as_ref().map(|cursor| cursor.runtime),
            already_delivered,
            remaining_capacity,
        )
        .await?
        {
            match runtime_item {
                RuntimePayloadItem::Durable(durable) => {
                    return self.push_durable_runtime_payloads(
                        durable.final_cursor,
                        durable.item_cursor,
                        durable.payloads,
                        durable.total,
                        durable.already_delivered,
                    );
                }
                RuntimePayloadItem::Live { cursor, payload } => {
                    return Ok(self.push_live_payload(cursor, payload));
                }
            }
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
    live: Option<EventProjectionCursor>,
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
            || parsed.live.is_some()
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
        live: None,
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
    if let Some(live) = cursor.live.as_ref()
        && &live.scope != projection_scope
    {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind: "projection_cursor",
            reason: "live cursor scope does not match subscription scope".to_string(),
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
    last_live_cursor: Option<EventCursor>,
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
            match envelope.as_ref() {
                ironclaw_event_streams::ProductProjectionEnvelope::ThreadUpdates(replay) => {
                    replay_payloads(
                        scope,
                        display_previews,
                        replay,
                        cursor,
                        expected_item,
                        already_delivered,
                        capacity,
                    )
                    .await
                }
                ironclaw_event_streams::ProductProjectionEnvelope::ThreadLiveUpdate(update) => {
                    live_update_payloads(scope, update, cursor, last_live_cursor)
                }
                _ => Err(internal_projection_error(
                    "unexpected projection update envelope",
                )),
            }
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

fn live_update_payloads(
    scope: &TurnScope,
    update: &ThreadLiveProjectionUpdate,
    cursor: EventProjectionCursor,
    last_live_cursor: Option<EventCursor>,
) -> RuntimePayloadItemResult {
    if last_live_cursor.is_some_and(|last| cursor.runtime <= last) {
        return Ok(None);
    }
    let items = product_items_for_live_update(update);
    if items.is_empty() {
        return Ok(None);
    }
    let state = ProductProjectionState::new(scope.thread_id.to_string(), items)?;
    Ok(Some(RuntimePayloadItem::Live {
        cursor,
        payload: ProductOutboundPayload::ProjectionUpdate { state },
    }))
}

#[derive(Debug)]
struct DurableRuntimePayloadItem {
    final_cursor: EventProjectionCursor,
    item_cursor: EventProjectionCursor,
    payloads: Vec<DeliveredRuntimePayload>,
    total: usize,
    already_delivered: usize,
}

#[derive(Debug)]
enum RuntimePayloadItem {
    Durable(DurableRuntimePayloadItem),
    Live {
        cursor: EventProjectionCursor,
        payload: ProductOutboundPayload,
    },
}

type RuntimePayloadItemResult = Result<Option<RuntimePayloadItem>, ProductAdapterError>;

fn durable_runtime_payload_item(
    final_cursor: EventProjectionCursor,
    item_cursor: EventProjectionCursor,
    payloads: Vec<DeliveredRuntimePayload>,
    total: usize,
    already_delivered: usize,
) -> RuntimePayloadItem {
    RuntimePayloadItem::Durable(DurableRuntimePayloadItem {
        final_cursor,
        item_cursor,
        payloads,
        total,
        already_delivered,
    })
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
    let total = all_payloads.total();
    let already_delivered =
        effective_runtime_payload_offset(already_delivered, expected_item, item_cursor.runtime);
    if already_delivered > total {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind: "projection_cursor",
            reason: "runtime delivery offset exceeds runtime item payload count".to_string(),
        });
    }
    let payloads = all_payloads.deliver_after(already_delivered, capacity);
    Ok(Some(durable_runtime_payload_item(
        cursor,
        item_cursor,
        payloads,
        total,
        already_delivered,
    )))
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
    let total = all_payloads.total();
    let already_delivered =
        effective_runtime_payload_offset(already_delivered, expected_item, item_cursor.runtime);
    if already_delivered > total {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind: "projection_cursor",
            reason: "runtime delivery offset exceeds runtime item payload count".to_string(),
        });
    }
    let payloads = all_payloads.deliver_after(already_delivered, capacity);
    Ok(Some(durable_runtime_payload_item(
        cursor,
        item_cursor,
        payloads,
        total,
        already_delivered,
    )))
}

#[cfg(test)]
struct RuntimePayloadItemInput {
    runs: Vec<RunStatusProjection>,
    capability_activities: Vec<CapabilityActivityProjection>,
    cursor: EventProjectionCursor,
    state_kind: StatePayloadKind,
}

#[derive(Clone, Copy)]
enum StatePayloadKind {
    Snapshot,
    Update,
}

#[cfg(test)]
async fn runtime_payloads_for_item(
    scope: &TurnScope,
    display_previews: &dyn CapabilityDisplayPreviewSource,
    input: RuntimePayloadItemInput,
    expected_item: Option<EventCursor>,
    already_delivered: usize,
    capacity: usize,
) -> Result<Option<DurableRuntimePayloadItem>, ProductAdapterError> {
    let RuntimePayloadItemInput {
        runs,
        capability_activities,
        cursor,
        state_kind,
    } = input;
    let snapshot = ProjectionSnapshot {
        timeline: ironclaw_event_projections::ThreadTimeline {
            entries: Vec::new(),
        },
        runs,
        capability_activities,
        next_cursor: cursor.clone(),
        truncated: false,
    };
    let item_cursor = snapshot_item_cursor(&snapshot, &cursor);
    let candidates = snapshot_payload_candidates(snapshot);
    let all_payloads =
        runtime_payloads_from_candidates(scope, display_previews, candidates, state_kind).await?;
    if all_payloads.is_empty() {
        return Ok(None);
    }
    let total = all_payloads.total();
    let already_delivered =
        effective_runtime_payload_offset(already_delivered, expected_item, item_cursor.runtime);
    if already_delivered > total {
        return Err(ProductAdapterError::InvalidIdentifier {
            kind: "projection_cursor",
            reason: "runtime delivery offset exceeds runtime item payload count".to_string(),
        });
    }
    let payloads = all_payloads.deliver_after(already_delivered, capacity);
    Ok(Some(DurableRuntimePayloadItem {
        final_cursor: cursor,
        item_cursor,
        payloads,
        total,
        already_delivered,
    }))
}

async fn runtime_payloads_from_candidates(
    scope: &TurnScope,
    display_previews: &dyn CapabilityDisplayPreviewSource,
    candidates: Vec<RuntimePayloadCandidate>,
    state_kind: StatePayloadKind,
) -> Result<RuntimePayloads, ProductAdapterError> {
    let resolutions = stream::iter(candidates)
        .map(|candidate| {
            runtime_payload_from_candidate(scope, display_previews, candidate, state_kind)
        })
        .buffered(16)
        .collect::<Vec<_>>()
        .await;
    RuntimePayloads::from_resolutions(resolutions)
}

async fn runtime_payload_from_candidate(
    scope: &TurnScope,
    display_previews: &dyn CapabilityDisplayPreviewSource,
    candidate: RuntimePayloadCandidate,
    state_kind: StatePayloadKind,
) -> Result<RuntimePayloadResolution, ProductAdapterError> {
    match candidate {
        RuntimePayloadCandidate::State { runs, .. } => {
            let state = run_status_projection_state(scope, runs)?
                .ok_or_else(|| internal_projection_error("missing run projection state"))?;
            let payload = match state_kind {
                StatePayloadKind::Snapshot => ProductOutboundPayload::ProjectionSnapshot { state },
                StatePayloadKind::Update => ProductOutboundPayload::ProjectionUpdate { state },
            };
            Ok(RuntimePayloadResolution::Payload(Box::new(payload)))
        }
        RuntimePayloadCandidate::CapabilityActivity(activity) => {
            CapabilityActivityView::new(CapabilityActivityViewInput {
                invocation_id: activity.invocation_id,
                turn_run_id: activity
                    .run_id
                    .map(|run_id| TurnRunId::from_uuid(run_id.as_uuid())),
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
            .map(Box::new)
            .map(RuntimePayloadResolution::Payload)
        }
        RuntimePayloadCandidate::CapabilityDisplayPreview(activity) => {
            match display_previews.preview_resolution(&activity).await {
                Ok(CapabilityDisplayPreviewResolution::Ready(preview)) => {
                    Ok(RuntimePayloadResolution::Payload(Box::new(
                        ProductOutboundPayload::CapabilityDisplayPreview(*preview),
                    )))
                }
                Ok(CapabilityDisplayPreviewResolution::Pending) => {
                    Ok(RuntimePayloadResolution::Pending)
                }
                Ok(CapabilityDisplayPreviewResolution::NotApplicable) => {
                    Ok(RuntimePayloadResolution::Empty)
                }
                Err(error) => {
                    tracing::debug!(
                        invocation_id = %activity.invocation_id,
                        capability_id = activity.capability_id.as_str(),
                        error = %error,
                        "capability display preview projection failed; continuing without preview"
                    );
                    Ok(RuntimePayloadResolution::Empty)
                }
            }
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
        .chain(
            replay
                .capability_activity_transitions
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

fn run_status_projection_state(
    scope: &TurnScope,
    runs: Vec<RunStatusProjection>,
) -> Result<Option<ProductProjectionState>, ProductAdapterError> {
    let items = runs
        .into_iter()
        .map(|run| ProductProjectionItem::RunStatus {
            run_id: TurnRunId::from_uuid(run.invocation_id.as_uuid()),
            status: run_status_wire(run.status).to_string(),
            failure_category: run_failure_category(&run),
            failure_summary: run_failure_summary(&run),
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        return Ok(None);
    }
    ProductProjectionState::new(scope.thread_id.to_string(), items).map(Some)
}

fn run_failure_category(run: &RunStatusProjection) -> Option<SanitizedFailure> {
    // Runtime replay categories intentionally use the event-projection namespace
    // (model_failed, dispatch_failed, process_killed, ...), while turn lifecycle
    // events use runner/driver failure reasons (lease_expired, driver_failed, ...).
    // Both are sanitized product categories; clients must treat the field as an
    // opaque category and prefer failure_summary for user-facing copy.
    matches!(
        run.status,
        RunProjectionStatus::Failed | RunProjectionStatus::Killed
    )
    .then(|| run.error_kind.clone())
    .flatten()
    .and_then(|category| SanitizedFailure::new(category).ok())
}

fn run_failure_summary(run: &RunStatusProjection) -> Option<String> {
    run_failure_category(run)
        .as_ref()
        .map(SanitizedFailure::category)
        .map(runtime_failure_summary_for_category)
        .map(str::to_string)
}

fn runtime_failure_summary_for_category(category: &str) -> &'static str {
    match category {
        "model_failed" => "The run failed while waiting for the model.",
        "dispatch_failed" => "The run failed while executing a capability.",
        "process_failed" => "The run failed while executing a runtime process.",
        "process_killed" => "The run stopped because its runtime process was killed.",
        "hook_failed" => "The run failed while evaluating a runtime hook.",
        "unknown" | "unclassified" => "The run failed for an unknown reason.",
        _ => "The run failed before producing a reply.",
    }
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
