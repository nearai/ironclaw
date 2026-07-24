// arch-exempt: large_file, mechanical InMemoryOutboundStateStore -> FilesystemOutboundStateStore<InMemoryBackend> §4.3 store consolidation, no logic change, plan #6168
use std::{
    sync::{Arc, atomic::AtomicU64},
    time::Duration,
};

use async_trait::async_trait;
use futures::{StreamExt, stream};
use ironclaw_event_projections::{
    CapabilityActivityProjection, CapabilityActivityStatus, EventProjectionService,
    ProjectionCursor as EventProjectionCursor, ProjectionReplay,
    ProjectionScope as EventProjectionScope, ProjectionSnapshot, ReplayEventProjectionService,
    RunProjectionStatus, RunStatusProjection,
};
use ironclaw_event_streams::{
    AllowAllProjectionAccessPolicy, EventStreamManager, InMemoryProjectionStreamAdmissionPolicy,
    InMemoryProjectionUpdateSource, NoExposureProjectionRedactionValidator,
    ProductProjectionEnvelope, ProjectionStreamError as EventProjectionStreamError,
    ProjectionStreamItem, ProjectionSubscribeRequest,
    ProjectionSubscription as EventProjectionSubscription, ProjectionTarget, ProjectionViewClass,
    SubscriberCapabilities, ThreadLiveProjectionUpdate,
};
use ironclaw_events::{DurableEventLog, EventCursor, EventStreamKey, ReadScope};
use ironclaw_filesystem::InMemoryBackend;
use ironclaw_first_party_extension_ports::SkillActivationObserver;
use ironclaw_host_api::UserId;
use ironclaw_outbound::FilesystemOutboundStateStore;
use ironclaw_product::{
    AdapterInstallationId, CapabilityActivityStatusView, CapabilityActivityView,
    CapabilityActivityViewInput, ExternalActorRef, ExternalConversationRef, ProductAdapterError,
    ProductAdapterId, ProductOutboundEnvelope, ProductOutboundPayload, ProductOutboundTarget,
    ProductProjectionItem, ProductProjectionState, ProductWorkflowRejectionKind,
    ProjectionCursor as ProductProjectionCursor, ProjectionStream, ProjectionStreamSubscription,
    ProjectionSubscriptionRequest, RedactedString,
};
use ironclaw_run_state::ApprovalRequestStore;
use ironclaw_turns::{
    ReplyTargetBindingRef, SanitizedFailure, TurnActor, TurnCoordinator, TurnError,
    TurnEventProjectionCursor, TurnEventProjectionSource, TurnEventSink, TurnLifecycleEvent,
    TurnRunId, TurnScope, TurnStatus, run_profile::LoopHostMilestoneSink,
};
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

mod display_preview;
mod live_progress;
mod runtime_replay;
mod turn_events;
use display_preview::{
    CapabilityDisplayPreviewResolution, CapabilityDisplayPreviewSource,
    NoopCapabilityDisplayPreviewSource,
};
use ironclaw_product::AuthChallengeProvider;
use live_progress::{
    LiveProgressMilestoneSink, LiveSkillActivationObserver, product_items_for_live_update,
};
// Crate-visible so the skill-learning sink can name the publisher type.
pub(crate) use live_progress::LiveProjectionPublisher;
use runtime_replay::{
    DeliveredRuntimePayload, RuntimePayloadCandidate, RuntimePayloadResolution, RuntimePayloads,
    replay_payload_candidates, snapshot_payload_candidates,
};
// Only the Slack delivery path (feature-gated) consumes this re-export.
pub(crate) use turn_events::approval_prompt_context_view;
use turn_events::{
    FailureExplanationProvider, ModelFailureExplanationProvider, TurnEventBridge, TurnEventDrain,
    TurnEventPayload, turn_status_wire,
};

pub(crate) use display_preview::{CapabilityDisplayPreviewResult, CapabilityDisplayPreviewStore};
#[cfg(test)]
pub(crate) use display_preview::{SANITIZE_JSON_MAX_DEPTH, sanitize_json_value, sanitize_text};

const PRODUCT_PROJECTION_PAGE_LIMIT: usize = 256;
const PRODUCT_RUNTIME_ITEM_MAX_PAYLOADS: usize = PRODUCT_PROJECTION_PAGE_LIMIT + 1;
const PRODUCT_PROJECTION_ADAPTER_ID: &str = "webui_v2";
const PRODUCT_PROJECTION_INSTALLATION_ID: &str = "webui_v2.local";
const TURN_EVENT_WAKE_BUFFER: usize = 256;
const PRODUCT_TERMINAL_TURN_LIVE_DRAIN_TIMEOUT: Duration = Duration::from_millis(50);

#[derive(Clone)]
pub(crate) struct RebornProjectionServices {
    event_stream_manager: Arc<EventStreamManager>,
    live_updates: Arc<InMemoryProjectionUpdateSource>,
    live_sequence: Arc<AtomicU64>,
    live_epoch: Arc<str>,
    turn_event_wake_source: Arc<TurnEventWakeSource>,
    turn_events: TurnEventBridge,
    approval_requests: Option<Arc<dyn ApprovalRequestStore>>,
    display_previews: Arc<dyn CapabilityDisplayPreviewSource>,
    product_reply_target_binding_ref: ReplyTargetBindingRef,
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

    pub(crate) fn product_event_stream(&self) -> Arc<dyn ProjectionStream> {
        Arc::new(ProductRuntimeProjectionStream {
            manager: Arc::clone(&self.event_stream_manager),
            turn_events: self.turn_events.clone(),
            turn_event_wake_source: Arc::clone(&self.turn_event_wake_source),
            auth_challenges: self.auth_challenges.clone(),
            display_previews: Arc::clone(&self.display_previews),
            reply_target_binding_ref: self.product_reply_target_binding_ref.clone(),
            live_epoch: Arc::clone(&self.live_epoch),
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
            Arc::clone(&self.live_sequence),
        ))
    }

    pub(crate) fn skill_activation_observer(
        &self,
        publisher: Arc<LiveProjectionPublisher>,
    ) -> Arc<dyn SkillActivationObserver> {
        Arc::new(LiveSkillActivationObserver::new(publisher))
    }

    pub(crate) fn turn_event_wake_sink(&self) -> Arc<dyn TurnEventSink> {
        Arc::new(TurnEventWakeSink {
            source: Arc::clone(&self.turn_event_wake_source),
        })
    }
}

pub(crate) fn build_reborn_projection_services(
    event_log: Arc<dyn DurableEventLog>,
    product_reply_target_binding_ref: ReplyTargetBindingRef,
) -> RebornProjectionServices {
    let projection: Arc<dyn EventProjectionService> =
        Arc::new(ReplayEventProjectionService::from_runtime_log(event_log));
    let live_updates = Arc::new(InMemoryProjectionUpdateSource::new(128));
    // One counter per projection-services bundle keeps all live publishers in
    // the same SSE cursor space; per-publisher counters can collide after a
    // durable cursor has advanced.
    let live_sequence = Arc::new(AtomicU64::new(0));
    let live_epoch: Arc<str> = Arc::from(Uuid::new_v4().to_string());
    let event_stream_manager = Arc::new(EventStreamManager::from_services(
        projection,
        Arc::new(AllowAllProjectionAccessPolicy),
        Arc::new(InMemoryProjectionStreamAdmissionPolicy::default()),
        live_updates.clone(),
        Arc::new(NoExposureProjectionRedactionValidator),
        // §4.3: the local-dev projection bundle's EventStreamManager keeps its
        // own ephemeral, volatile outbound-delivery bookkeeping — the drop-in
        // for the deleted throwaway `InMemoryOutboundStateStore::default()`.
        // `wrap_scoped` over a fresh `InMemoryBackend` mounts `/outbound`; the
        // durable outbound store is composed separately in the factory.
        // composition-owned construction site.
        {
            #[allow(clippy::disallowed_methods)]
            Arc::new(FilesystemOutboundStateStore::new(crate::wrap_scoped(
                Arc::new(InMemoryBackend::new()),
            )))
        },
    ));
    RebornProjectionServices {
        event_stream_manager,
        live_updates,
        live_sequence,
        live_epoch,
        turn_event_wake_source: Arc::new(TurnEventWakeSource::new()),
        turn_events: TurnEventBridge::default(),
        approval_requests: None,
        display_previews: Arc::new(NoopCapabilityDisplayPreviewSource),
        product_reply_target_binding_ref,
        auth_challenges: None,
    }
}

#[derive(Debug, Clone)]
struct TurnEventWake {
    scope: TurnScope,
    owner_user_id: Option<UserId>,
}

struct TurnEventWakeSource {
    sender: broadcast::Sender<TurnEventWake>,
}

impl TurnEventWakeSource {
    fn new() -> Self {
        let (sender, _) = broadcast::channel(TURN_EVENT_WAKE_BUFFER);
        Self { sender }
    }

    fn subscribe(&self) -> broadcast::Receiver<TurnEventWake> {
        self.sender.subscribe()
    }

    fn publish(&self, event: &TurnLifecycleEvent) {
        let _ = self.sender.send(TurnEventWake {
            scope: event.scope.clone(),
            owner_user_id: event.owner_user_id.clone(),
        });
    }
}

struct TurnEventWakeSink {
    source: Arc<TurnEventWakeSource>,
}

#[async_trait]
impl TurnEventSink for TurnEventWakeSink {
    async fn publish(&self, event: TurnLifecycleEvent) -> Result<(), TurnError> {
        self.source.publish(&event);
        Ok(())
    }
}

fn turn_wake_matches_request(
    wake: &TurnEventWake,
    request: &ProjectionSubscriptionRequest,
) -> bool {
    wake.scope == request.scope
        && wake
            .owner_user_id
            .as_ref()
            .is_none_or(|owner_user_id| owner_user_id == &request.actor.user_id)
}

/// WebUI bridge over the shared EventStreamManager.
///
/// This exposes runtime projection payloads that WebChat v2 has first-class
/// SSE frames for: run status and capability activity. Timeline content stays
/// behind the WebUI timeline facade until the browser event schema grows a
/// first-class timeline-entry mapper.
#[derive(Clone)]
struct ProductRuntimeProjectionStream {
    manager: Arc<EventStreamManager>,
    turn_events: TurnEventBridge,
    turn_event_wake_source: Arc<TurnEventWakeSource>,
    auth_challenges: Option<Arc<dyn AuthChallengeProvider>>,
    display_previews: Arc<dyn CapabilityDisplayPreviewSource>,
    reply_target_binding_ref: ReplyTargetBindingRef,
    live_epoch: Arc<str>,
}

#[async_trait]
impl ProjectionStream for ProductRuntimeProjectionStream {
    fn supports_subscription(&self) -> bool {
        true
    }

    async fn subscribe(
        &self,
        request: ProjectionSubscriptionRequest,
    ) -> Result<ProjectionStreamSubscription, ProductAdapterError> {
        let (subscription, origin_cursor) = self.runtime_subscription(&request).await?;
        let (sender, receiver) = mpsc::channel(PRODUCT_PROJECTION_PAGE_LIMIT);
        let stream = self.clone();
        tokio::spawn(async move {
            stream
                .forward_subscription(request, subscription, origin_cursor, sender)
                .await;
        });
        Ok(ProjectionStreamSubscription::new(receiver))
    }

    async fn drain(
        &self,
        request: ProjectionSubscriptionRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, ProductAdapterError> {
        let (mut subscription, origin_cursor) = self.runtime_subscription(&request).await?;

        let is_resuming_runtime_payloads = origin_cursor.runtime_payloads_delivered > 0;
        let mut batch = ProductSurfaceProjectionBatch::new(origin_cursor);
        if let Some(item) = subscription.next().await {
            let buffered = if is_resuming_runtime_payloads {
                Vec::new()
            } else {
                collect_buffered_runtime_items(&mut subscription)
            };
            let keep_consuming = push_ordered_initial_runtime_items(
                &mut batch,
                item,
                buffered,
                &request.scope,
                self.display_previews.as_ref(),
            )
            .await?;
            if keep_consuming && !is_resuming_runtime_payloads {
                consume_buffered_runtime_items(
                    &mut subscription,
                    &mut batch,
                    &request.scope,
                    self.display_previews.as_ref(),
                )
                .await?;
            }
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

        self.append_turn_events(&mut batch, Some(&mut subscription), &request)
            .await?;
        self.batch_into_outbound(batch, &request)
    }
}

impl ProductRuntimeProjectionStream {
    async fn runtime_subscription(
        &self,
        request: &ProjectionSubscriptionRequest,
    ) -> Result<(EventProjectionSubscription, ProductSurfaceProjectionCursor), ProductAdapterError>
    {
        let projection_scope = runtime_projection_scope(&request.actor, &request.scope);
        let mut origin_cursor = request
            .after_cursor
            .clone()
            .map(|cursor| parse_product_projection_cursor(cursor.as_str()))
            .transpose()?
            .unwrap_or_default();
        validate_product_projection_cursor_scope(
            &origin_cursor,
            &request.scope,
            &projection_scope,
        )?;
        // Live projection updates are process-local and their numeric sequence
        // restarts from zero with each projection-services bundle. A browser
        // can retain an otherwise valid composite cursor across a deployment;
        // carrying that prior process's live floor forward would suppress new
        // live updates until the restarted counter overtook it. Keep the
        // durable runtime/turn positions, but rebase only the volatile live
        // component when the producing process changes.
        if origin_cursor.live.is_some()
            && origin_cursor.live_epoch.as_deref() != Some(self.live_epoch.as_ref())
        {
            origin_cursor.live = None;
        }
        origin_cursor.live_epoch = Some(self.live_epoch.to_string());
        let subscription = self
            .manager
            .subscribe(ProjectionSubscribeRequest {
                actor: request.actor.clone(),
                scope: projection_scope.clone(),
                view: ProjectionViewClass::ProductThread,
                target: ProjectionTarget::Thread {
                    thread_id: request.scope.thread_id.clone(),
                },
                after_cursor: origin_cursor.runtime.clone(),
                limit: PRODUCT_PROJECTION_PAGE_LIMIT,
                capabilities: SubscriberCapabilities::default(),
            })
            .await
            .map_err(map_event_stream_error)?;
        Ok((subscription, origin_cursor))
    }

    async fn forward_subscription(
        self,
        request: ProjectionSubscriptionRequest,
        mut subscription: EventProjectionSubscription,
        origin_cursor: ProductSurfaceProjectionCursor,
        sender: mpsc::Sender<Result<ProductOutboundEnvelope, ProductAdapterError>>,
    ) {
        let mut cursor = origin_cursor;
        let is_resuming_runtime_payloads = cursor.runtime_payloads_delivered > 0;
        let mut turn_wakes = self.turn_event_wake_source.subscribe();
        let first = tokio::select! {
            _ = sender.closed() => return,
            item = subscription.next() => {
                let Some(item) = item else {
                    return;
                };
                item
            }
        };
        let mut batch = ProductSurfaceProjectionBatch::new(cursor.clone());
        let buffered = if is_resuming_runtime_payloads {
            Vec::new()
        } else {
            collect_buffered_runtime_items(&mut subscription)
        };
        let first_result = push_ordered_initial_runtime_items(
            &mut batch,
            first,
            buffered,
            &request.scope,
            self.display_previews.as_ref(),
        )
        .await;
        let mut keep_consuming = match first_result {
            Ok(keep_consuming) => keep_consuming,
            Err(error) => {
                send_projection_subscription_error(&sender, error).await;
                return;
            }
        };
        if keep_consuming
            && !is_resuming_runtime_payloads
            && let Err(error) = consume_buffered_runtime_items(
                &mut subscription,
                &mut batch,
                &request.scope,
                self.display_previews.as_ref(),
            )
            .await
        {
            send_projection_subscription_error(&sender, error).await;
            return;
        }
        if let Err(error) = self
            .append_turn_events(&mut batch, Some(&mut subscription), &request)
            .await
        {
            send_projection_subscription_error(&sender, error).await;
            return;
        }
        if !self
            .send_subscription_batch(batch, &request, &sender, &mut cursor)
            .await
        {
            return;
        }
        if !keep_consuming {
            return;
        }

        loop {
            tokio::select! {
                _ = sender.closed() => {
                    return;
                }
                item = subscription.next() => {
                    let Some(item) = item else {
                        return;
                    };
                    let mut batch = ProductSurfaceProjectionBatch::new(cursor.clone());
                    keep_consuming = match push_runtime_item(
                        &mut batch,
                        item,
                        &request.scope,
                        self.display_previews.as_ref(),
                    )
                    .await
                    {
                        Ok(keep_consuming) => keep_consuming,
                        Err(error) => {
                            send_projection_subscription_error(&sender, error).await;
                            return;
                        }
                    };
                    if let Err(error) = self
                        .append_turn_events(&mut batch, Some(&mut subscription), &request)
                        .await
                    {
                        send_projection_subscription_error(&sender, error).await;
                        return;
                    }
                    if !self
                        .send_subscription_batch(batch, &request, &sender, &mut cursor)
                        .await
                    {
                        return;
                    }
                    if !keep_consuming {
                        return;
                    }
                }
                wake = turn_wakes.recv() => {
                    match wake {
                        Ok(wake) if !turn_wake_matches_request(&wake, &request) => {
                            continue;
                        }
                        Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => {
                            return;
                        }
                    }

                    let mut batch = ProductSurfaceProjectionBatch::new(cursor.clone());
                    if let Err(error) = consume_buffered_runtime_items(
                        &mut subscription,
                        &mut batch,
                        &request.scope,
                        self.display_previews.as_ref(),
                    )
                    .await
                    {
                        send_projection_subscription_error(&sender, error).await;
                        return;
                    }
                    if let Err(error) = self
                        .append_turn_events(&mut batch, Some(&mut subscription), &request)
                        .await
                    {
                        send_projection_subscription_error(&sender, error).await;
                        return;
                    }
                    if !self
                        .send_subscription_batch(batch, &request, &sender, &mut cursor)
                        .await
                    {
                        return;
                    }
                }
            }
        }
    }

    async fn append_turn_events(
        &self,
        batch: &mut ProductSurfaceProjectionBatch,
        mut subscription: Option<&mut EventProjectionSubscription>,
        request: &ProjectionSubscriptionRequest,
    ) -> Result<(), ProductAdapterError> {
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
        if turn_drain_has_terminal_run_status(&turn_drain)
            && let Some(subscription) = subscription.as_mut()
        {
            drain_runtime_items_before_terminal_turn(
                subscription,
                batch,
                &request.scope,
                self.display_previews.as_ref(),
            )
            .await?;
        }
        for TurnEventPayload {
            cursor: turn_cursor,
            payload,
        } in turn_drain.payloads
        {
            batch.push_turn(turn_cursor, payload);
        }
        if let Some(next_cursor) = turn_drain.next_cursor
            && turn_cursor_advances(batch.cursor().turn.as_ref(), &next_cursor)
        {
            batch.push_turn(next_cursor, ProductOutboundPayload::KeepAlive);
        }
        Ok(())
    }

    fn batch_into_outbound(
        &self,
        batch: ProductSurfaceProjectionBatch,
        request: &ProjectionSubscriptionRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, ProductAdapterError> {
        batch
            .into_payloads()
            .map(|(cursor, payload)| {
                envelope_to_outbound(
                    product_cursor_from_projection_cursor(&cursor)?,
                    payload,
                    &request.scope,
                    &request.actor,
                    &self.reply_target_binding_ref,
                )
            })
            .collect()
    }

    async fn send_subscription_batch(
        &self,
        batch: ProductSurfaceProjectionBatch,
        request: &ProjectionSubscriptionRequest,
        sender: &mpsc::Sender<Result<ProductOutboundEnvelope, ProductAdapterError>>,
        cursor: &mut ProductSurfaceProjectionCursor,
    ) -> bool {
        for (next_cursor, payload) in batch.into_payloads() {
            let projection_cursor = match product_cursor_from_projection_cursor(&next_cursor) {
                Ok(cursor) => cursor,
                Err(error) => {
                    send_projection_subscription_error(sender, error).await;
                    return false;
                }
            };
            let envelope = match envelope_to_outbound(
                projection_cursor,
                payload,
                &request.scope,
                &request.actor,
                &self.reply_target_binding_ref,
            ) {
                Ok(envelope) => envelope,
                Err(error) => {
                    send_projection_subscription_error(sender, error).await;
                    return false;
                }
            };
            *cursor = next_cursor;
            if sender.send(Ok(envelope)).await.is_err() {
                return false;
            }
        }
        true
    }
}

async fn send_projection_subscription_error(
    sender: &mpsc::Sender<Result<ProductOutboundEnvelope, ProductAdapterError>>,
    error: ProductAdapterError,
) {
    let _ = sender.send(Err(error)).await;
}

async fn consume_buffered_runtime_items(
    subscription: &mut EventProjectionSubscription,
    batch: &mut ProductSurfaceProjectionBatch,
    scope: &TurnScope,
    display_previews: &dyn CapabilityDisplayPreviewSource,
) -> Result<(), ProductAdapterError> {
    for item in collect_buffered_runtime_items(subscription) {
        if !push_runtime_item(batch, item, scope, display_previews).await? {
            break;
        }
    }
    Ok(())
}

fn collect_buffered_runtime_items(
    subscription: &mut EventProjectionSubscription,
) -> Vec<ProjectionStreamItem> {
    let mut items = Vec::new();
    for _ in 0..PRODUCT_PROJECTION_PAGE_LIMIT {
        let Some(item) = subscription.try_next_buffered() else {
            break;
        };
        items.push(item);
    }
    items
}

async fn drain_runtime_items_before_terminal_turn(
    subscription: &mut EventProjectionSubscription,
    batch: &mut ProductSurfaceProjectionBatch,
    scope: &TurnScope,
    display_previews: &dyn CapabilityDisplayPreviewSource,
) -> Result<(), ProductAdapterError> {
    consume_buffered_runtime_items(subscription, batch, scope, display_previews).await?;
    if !batch.has_runtime_payload_capacity() {
        return Ok(());
    }
    let item = tokio::time::timeout(
        PRODUCT_TERMINAL_TURN_LIVE_DRAIN_TIMEOUT,
        subscription.next(),
    )
    .await
    .ok()
    .flatten();
    if let Some(item) = item {
        let keep_consuming = push_runtime_item(batch, item, scope, display_previews).await?;
        if keep_consuming {
            consume_buffered_runtime_items(subscription, batch, scope, display_previews).await?;
        }
    }
    Ok(())
}

async fn push_ordered_initial_runtime_items(
    batch: &mut ProductSurfaceProjectionBatch,
    first: ProjectionStreamItem,
    buffered: Vec<ProjectionStreamItem>,
    scope: &TurnScope,
    display_previews: &dyn CapabilityDisplayPreviewSource,
) -> Result<bool, ProductAdapterError> {
    // Live progress and durable state use independent cursor rails. When both
    // are buffered, drain live updates before a terminal durable run status so
    // the browser can render assistant text/progress before settling the run.
    if durable_item_has_terminal_run_status(&first)
        && buffered.iter().any(projection_item_is_live_update)
    {
        let (live_updates, other_items): (Vec<_>, Vec<_>) = buffered
            .into_iter()
            .partition(projection_item_is_live_update);
        for item in live_updates {
            if !push_runtime_item(batch, item, scope, display_previews).await? {
                return Ok(false);
            }
        }
        if !push_runtime_item(batch, first, scope, display_previews).await? {
            return Ok(false);
        }
        for item in other_items {
            if !push_runtime_item(batch, item, scope, display_previews).await? {
                return Ok(false);
            }
        }
        return Ok(true);
    }

    if !push_runtime_item(batch, first, scope, display_previews).await? {
        return Ok(false);
    }
    for item in buffered {
        if !push_runtime_item(batch, item, scope, display_previews).await? {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn push_runtime_item(
    batch: &mut ProductSurfaceProjectionBatch,
    item: ProjectionStreamItem,
    scope: &TurnScope,
    display_previews: &dyn CapabilityDisplayPreviewSource,
) -> Result<bool, ProductAdapterError> {
    if !batch.has_runtime_payload_capacity() {
        return Ok(false);
    }
    batch.push_runtime_item(item, scope, display_previews).await
}

fn turn_drain_has_terminal_run_status(drain: &TurnEventDrain) -> bool {
    drain
        .payloads
        .iter()
        .any(|payload| outbound_payload_has_terminal_run_status(&payload.payload))
}

fn outbound_payload_has_terminal_run_status(payload: &ProductOutboundPayload) -> bool {
    let state = match payload {
        ProductOutboundPayload::ProjectionSnapshot { state }
        | ProductOutboundPayload::ProjectionUpdate { state } => state,
        _ => return false,
    };
    state.items.iter().any(|item| {
        matches!(
            item,
            ProductProjectionItem::RunStatus { status, .. }
                if product_run_status_is_terminal(status)
        )
    })
}

fn product_run_status_is_terminal(status: &str) -> bool {
    const TERMINAL_RUNTIME_STATUSES: &[RunProjectionStatus] = &[
        RunProjectionStatus::Completed,
        RunProjectionStatus::Cancelled,
        RunProjectionStatus::Failed,
        RunProjectionStatus::Killed,
    ];
    const TERMINAL_TURN_STATUSES: &[TurnStatus] = &[
        TurnStatus::Completed,
        TurnStatus::Cancelled,
        TurnStatus::Failed,
    ];
    TERMINAL_RUNTIME_STATUSES
        .iter()
        .any(|terminal| status == run_status_wire(*terminal))
        || TERMINAL_TURN_STATUSES
            .iter()
            .any(|terminal| status == turn_status_wire(*terminal))
}

fn projection_item_is_live_update(item: &ProjectionStreamItem) -> bool {
    matches!(
        item,
        ProjectionStreamItem::Update(envelope)
            if matches!(envelope.as_ref(), ProductProjectionEnvelope::ThreadLiveUpdate(_))
    )
}

fn durable_item_has_terminal_run_status(item: &ProjectionStreamItem) -> bool {
    match item {
        ProjectionStreamItem::Snapshot(envelope) => envelope_has_terminal_run_status(envelope),
        ProjectionStreamItem::Update(envelope) => envelope_has_terminal_run_status(envelope),
        ProjectionStreamItem::RebaseRequired { snapshot, .. } => {
            envelope_has_terminal_run_status(snapshot)
        }
        ProjectionStreamItem::Lagged { .. } | ProjectionStreamItem::KeepAlive => false,
    }
}

fn envelope_has_terminal_run_status(envelope: &ProductProjectionEnvelope) -> bool {
    let runs = match envelope {
        ProductProjectionEnvelope::ThreadSnapshot(snapshot) => &snapshot.runs,
        ProductProjectionEnvelope::ThreadUpdates(replay) => &replay.runs,
        ProductProjectionEnvelope::ThreadLiveUpdate(_)
        | ProductProjectionEnvelope::DeliveryStatus(_)
        | ProductProjectionEnvelope::Debug(_) => return false,
    };
    runs.iter()
        .any(|run| run.status != RunProjectionStatus::Running)
}

struct ProductSurfaceProjectionBatch {
    cursor: ProductSurfaceProjectionCursor,
    pending_runtime_cursor_advance: Option<EventProjectionCursor>,
    runtime_payloads_pushed: usize,
    payloads: Vec<(ProductSurfaceProjectionCursor, ProductOutboundPayload)>,
}

impl ProductSurfaceProjectionBatch {
    fn new(cursor: ProductSurfaceProjectionCursor) -> Self {
        Self {
            cursor,
            pending_runtime_cursor_advance: None,
            runtime_payloads_pushed: 0,
            payloads: Vec::new(),
        }
    }

    fn cursor(&self) -> &ProductSurfaceProjectionCursor {
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
            self.push_runtime_or_live(ProductOutboundPayload::KeepAlive);
            return Ok(true);
        }

        let remaining_capacity =
            PRODUCT_RUNTIME_ITEM_MAX_PAYLOADS.saturating_sub(self.runtime_payloads_pushed);
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
            self.push_runtime_or_live(payload);
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
        self.push_runtime_or_live(payload);
        true
    }

    fn push_runtime_cursor_advance(&mut self, cursor: EventProjectionCursor) -> bool {
        if cursor.runtime.as_u64() == 0 {
            return true;
        }
        if self.runtime_cursor_covers(&cursor) {
            return true;
        }
        self.defer_runtime_cursor_advance(cursor);
        true
    }

    fn runtime_cursor_covers(&self, cursor: &EventProjectionCursor) -> bool {
        self.cursor
            .runtime
            .as_ref()
            .or(self.pending_runtime_cursor_advance.as_ref())
            .is_some_and(|current| current.runtime >= cursor.runtime)
    }

    fn defer_runtime_cursor_advance(&mut self, cursor: EventProjectionCursor) {
        self.pending_runtime_cursor_advance = Some(cursor);
    }

    fn flush_pending_runtime_cursor_advance(&mut self) {
        let Some(cursor) = self.pending_runtime_cursor_advance.take() else {
            return;
        };
        self.cursor.runtime = Some(cursor);
        self.cursor.runtime_item = None;
        self.cursor.runtime_payloads_delivered = 0;
        self.payloads
            .push((self.cursor.clone(), ProductOutboundPayload::KeepAlive));
    }

    async fn push_runtime_item(
        &mut self,
        item: ProjectionStreamItem,
        scope: &TurnScope,
        display_previews: &dyn CapabilityDisplayPreviewSource,
    ) -> Result<bool, ProductAdapterError> {
        let already_delivered = self.cursor.runtime_payloads_delivered;
        let remaining_capacity =
            PRODUCT_RUNTIME_ITEM_MAX_PAYLOADS.saturating_sub(self.runtime_payloads_pushed);
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
                    let durable = *durable;
                    return self.push_durable_runtime_payloads(
                        durable.final_cursor,
                        durable.item_cursor,
                        durable.payloads,
                        durable.total,
                        durable.already_delivered,
                    );
                }
                RuntimePayloadItem::Live { cursor, payload } => {
                    return Ok(self.push_live_payload(cursor, *payload));
                }
                RuntimePayloadItem::CursorAdvance { cursor } => {
                    return Ok(self.push_runtime_cursor_advance(cursor));
                }
            }
        }
        Ok(true)
    }

    fn has_runtime_payload_capacity(&self) -> bool {
        self.runtime_payloads_pushed < PRODUCT_RUNTIME_ITEM_MAX_PAYLOADS
    }

    fn push_turn(&mut self, cursor: TurnEventProjectionCursor, payload: ProductOutboundPayload) {
        self.cursor.turn = Some(cursor);
        self.push_preserving_runtime_cursor_advance(payload);
    }

    fn push_runtime_or_live(&mut self, payload: ProductOutboundPayload) {
        self.pending_runtime_cursor_advance = None;
        self.push_preserving_runtime_cursor_advance(payload);
    }

    fn push_preserving_runtime_cursor_advance(&mut self, payload: ProductOutboundPayload) {
        self.payloads.push((self.cursor.clone(), payload));
    }

    fn into_payloads(
        mut self,
    ) -> impl Iterator<Item = (ProductSurfaceProjectionCursor, ProductOutboundPayload)> {
        self.flush_pending_runtime_cursor_advance();
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

fn turn_cursor_advances(
    current: Option<&TurnEventProjectionCursor>,
    next: &TurnEventProjectionCursor,
) -> bool {
    current.is_none_or(|current| next.event > current.event)
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct ProductSurfaceProjectionCursor {
    runtime: Option<EventProjectionCursor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    live: Option<EventProjectionCursor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    live_epoch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    runtime_item: Option<EventCursor>,
    turn: Option<TurnEventProjectionCursor>,
    #[serde(default, skip_serializing_if = "is_zero")]
    runtime_payloads_delivered: usize,
}

fn is_zero(value: &usize) -> bool {
    *value == 0
}

fn parse_product_projection_cursor(
    cursor: &str,
) -> Result<ProductSurfaceProjectionCursor, ProductAdapterError> {
    if let Ok(parsed) = serde_json::from_str::<ProductSurfaceProjectionCursor>(cursor)
        && (parsed.runtime.is_some()
            || parsed.live.is_some()
            || parsed.turn.is_some()
            || parsed.runtime_payloads_delivered > 0)
    {
        if parsed.runtime_payloads_delivered > PRODUCT_RUNTIME_ITEM_MAX_PAYLOADS + 1 {
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
            reason: "must be a product projection cursor".to_string(),
        }
    })?;
    Ok(ProductSurfaceProjectionCursor {
        runtime: Some(runtime),
        live: None,
        live_epoch: None,
        runtime_item: None,
        turn: None,
        runtime_payloads_delivered: 0,
    })
}

fn validate_product_projection_cursor_scope(
    cursor: &ProductSurfaceProjectionCursor,
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

fn product_cursor_from_projection_cursor(
    cursor: &ProductSurfaceProjectionCursor,
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
                    live_update_payloads(scope, display_previews, update, cursor, last_live_cursor)
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
    display_previews: &dyn CapabilityDisplayPreviewSource,
    update: &ThreadLiveProjectionUpdate,
    cursor: EventProjectionCursor,
    last_live_cursor: Option<EventCursor>,
) -> RuntimePayloadItemResult {
    if last_live_cursor.is_some_and(|last| cursor.runtime <= last) {
        return Ok(None);
    }
    let items = product_items_for_live_update(display_previews, update);
    if items.is_empty() {
        return Ok(None);
    }
    let state = ProductProjectionState::new(scope.thread_id.to_string(), items)?;
    Ok(Some(RuntimePayloadItem::Live {
        cursor,
        payload: Box::new(ProductOutboundPayload::ProjectionUpdate { state }),
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
    Durable(Box<DurableRuntimePayloadItem>),
    Live {
        cursor: EventProjectionCursor,
        payload: Box<ProductOutboundPayload>,
    },
    CursorAdvance {
        cursor: EventProjectionCursor,
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
    RuntimePayloadItem::Durable(Box::new(DurableRuntimePayloadItem {
        final_cursor,
        item_cursor,
        payloads,
        total,
        already_delivered,
    }))
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
        return Ok(Some(RuntimePayloadItem::CursorAdvance { cursor }));
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
        return Ok(Some(RuntimePayloadItem::CursorAdvance { cursor }));
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
            let activity_order = activity.activity_order_cursor().as_u64();
            let error_detail =
                capability_activity_runtime_error_detail(&activity, display_previews);
            // Surface the staged input on the still-running activity frame so
            // the row shows `tool   <arg>` (and a populated Parameters tab)
            // live, instead of a bare tool name until the result lands.
            let running = display_previews.running_input(activity.invocation_id);
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
                // Runtime activity transitions can reach the browser before
                // the separate display-preview payload is delivered. Prefer
                // the durable event summary, then fall back to a staged
                // failure preview so the live card shows the same
                // host-authored copy as the refresh path.
                error_detail,
                subtitle: running.as_ref().and_then(|input| input.subtitle.clone()),
                input_summary: running.and_then(|input| input.input_summary),
                updated_at: activity.updated_at,
                activity_order: Some(activity_order),
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
            // Runtime-replay projections have no retryability signal; the
            // turn-lifecycle projection is the source of truth for it.
            retryable: None,
        })
        .collect::<Vec<_>>();
    if items.is_empty() {
        return Ok(None);
    }
    ProductProjectionState::new(scope.thread_id.to_string(), items).map(Some)
}

fn run_failure_category(run: &RunStatusProjection) -> Option<SanitizedFailure> {
    // `error_kind` is a sanitized product category sourced from the runtime
    // event log (see `ironclaw_event_projections::apply_run_event`). At
    // `Failed`/`Killed` status its concrete values are the dispatcher error
    // codes (`missing_runtime_backend`, `unknown_capability`, ...), the
    // `LoopFailureKind` codes (`model_error`, `iteration_limit`, ...), or the
    // process fallback `unknown`. Clients must treat the field as an opaque
    // category and prefer `failure_summary` for user-facing copy.
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
    // `category` comes from `RunStatusProjection.error_kind` (see
    // `run_failure_category`). The only sanitized value that resolves to a
    // dedicated message is the process fallback `unknown`; every other
    // produced value (dispatcher codes, `LoopFailureKind` codes) intentionally
    // falls through to the generic summary.
    match category {
        "unknown" => "The run failed for an unknown reason.",
        _ => "The run failed before producing a reply.",
    }
}

fn capability_activity_runtime_error_detail(
    activity: &CapabilityActivityProjection,
    display_previews: &dyn CapabilityDisplayPreviewSource,
) -> Option<String> {
    if !matches!(
        activity.status,
        CapabilityActivityStatus::Failed | CapabilityActivityStatus::Killed
    ) {
        return None;
    }
    activity
        .error_detail
        .clone()
        .or_else(|| display_previews.failure_error_detail(activity))
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
    let adapter_id = ProductAdapterId::new(PRODUCT_PROJECTION_ADAPTER_ID)?;
    let installation_id = AdapterInstallationId::new(PRODUCT_PROJECTION_INSTALLATION_ID)?;
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
