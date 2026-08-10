//! Best-effort forwarding of live model text into a disposable channel preview.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use ironclaw_extension_contracts::channel_adapter::{OutboundPart, ProgressivePreviewPart};
use ironclaw_extension_contracts::external::{ExternalActorRef, ExternalConversationRef};
use ironclaw_outbound::{
    CommunicationDeliveryIntent, CommunicationDeliveryResolutionRequest, CommunicationModality,
    OutboundPolicyService, PrepareCommunicationDeliveryRequest, ProjectionUpdateRef,
    RunNotificationContext, RunNotificationEventKind, RunNotificationOrigin, SourceRouteContext,
};
use ironclaw_product_contracts::outbound::{
    ProductOutboundEnvelope, ProductOutboundPayload, ProductProjectionItem,
};
use ironclaw_product_contracts::projection::{
    ProjectionStreamSubscription, ProjectionSubscriptionRequest,
};
use ironclaw_threads::ThreadScope;
use ironclaw_turns::{ReplyTargetBindingRef, TurnActor, TurnRunId, TurnScope};
use tokio::sync::oneshot;

use super::observer::{AllowNoProjectionAccess, ObservedReplyTargetAuthority};
use super::{PostedWorkingNotice, RunDeliveryServices};
use crate::delivery_coordinator::{
    CoordinatedDeliveryOutcome, CoordinatedDeliveryRequest, DeliveryIntent,
};

const FIRST_PREVIEW_UPDATE_DELAY: Duration = Duration::from_millis(150);
const PREVIEW_UPDATE_INTERVAL: Duration = Duration::from_secs(1);
const PREVIEW_SUBSCRIPTION_TIMEOUT: Duration = Duration::from_secs(1);
const PREVIEW_SHUTDOWN_DRAIN_TIMEOUT: Duration = Duration::from_millis(150);

pub(crate) struct PreviewForwarderHandle {
    shutdown: Option<oneshot::Sender<()>>,
    join: Option<tokio::task::JoinHandle<()>>,
}

impl PreviewForwarderHandle {
    pub(crate) async fn shutdown(mut self) {
        if let Some(sender) = self.shutdown.take() {
            let _ = sender.send(());
        }
        if let Some(join) = self.join.take() {
            let _ = join.await;
        }
    }
}

pub(crate) struct PreviewSourceRoute {
    pub(crate) thread_scope: ThreadScope,
    pub(crate) reply_target: ReplyTargetBindingRef,
    pub(crate) conversation: ExternalConversationRef,
    pub(crate) actor_ref: ExternalActorRef,
}

pub(crate) async fn spawn_preview_forwarder(
    services: Arc<RunDeliveryServices>,
    scope: TurnScope,
    actor: TurnActor,
    run_id: TurnRunId,
    notice: PostedWorkingNotice,
    route: PreviewSourceRoute,
) -> Option<PreviewForwarderHandle> {
    let subscription = match tokio::time::timeout(
        PREVIEW_SUBSCRIPTION_TIMEOUT,
        services
            .projection_stream
            .subscribe(ProjectionSubscriptionRequest {
                actor: actor.clone(),
                scope: scope.clone(),
                after_cursor: None,
            }),
    )
    .await
    {
        Ok(Ok(subscription)) => subscription,
        Ok(Err(error)) => {
            tracing::debug!(
                target: "ironclaw::reborn::run_delivery",
                %run_id,
                %error,
                "progressive preview subscription unavailable"
            );
            return None;
        }
        Err(_) => {
            tracing::debug!(
                target: "ironclaw::reborn::run_delivery",
                %run_id,
                "progressive preview subscription timed out"
            );
            return None;
        }
    };
    tracing::debug!(
        target: "ironclaw::reborn::run_delivery",
        %run_id,
        "progressive preview subscription ready"
    );
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let join = tokio::spawn(async move {
        forward_loop(
            PreviewForwarder {
                services,
                scope,
                actor,
                run_id,
                notice,
                route,
            },
            subscription,
            shutdown_rx,
        )
        .await;
    });
    Some(PreviewForwarderHandle {
        shutdown: Some(shutdown_tx),
        join: Some(join),
    })
}

struct PreviewForwarder {
    services: Arc<RunDeliveryServices>,
    scope: TurnScope,
    actor: TurnActor,
    run_id: TurnRunId,
    notice: PostedWorkingNotice,
    route: PreviewSourceRoute,
}

async fn forward_loop(
    forwarder: PreviewForwarder,
    mut subscription: ProjectionStreamSubscription,
    mut shutdown: oneshot::Receiver<()>,
) {
    let mut bodies = Vec::new();
    let mut accepted = String::new();
    let mut current = String::new();
    let mut sequence = 0_u64;
    let mut received_text_updates = 0_u64;
    let mut delivered_updates = 0_u64;
    let mut last_update_at = None;
    let mut next_update_at = None;

    loop {
        let update_timer =
            tokio::time::sleep_until(next_update_at.unwrap_or_else(tokio::time::Instant::now));
        tokio::pin!(update_timer);
        tokio::select! {
            _ = &mut shutdown => {
                let deadline = tokio::time::Instant::now() + PREVIEW_SHUTDOWN_DRAIN_TIMEOUT;
                loop {
                    let item = match tokio::time::timeout_at(deadline, subscription.next()).await {
                        Ok(item) => item,
                        Err(_) => break,
                    };
                    match item {
                        Some(Ok(envelope)) => {
                            if let Some(text) = live_cumulative_text(
                                &envelope,
                                forwarder.run_id,
                                &mut bodies,
                            ) {
                                current = text;
                                received_text_updates = received_text_updates.saturating_add(1);
                            }
                        }
                        Some(Err(error)) => {
                            tracing::debug!(
                                target: "ironclaw::reborn::run_delivery",
                                run_id = %forwarder.run_id,
                                %error,
                                "progressive preview update unavailable during shutdown"
                            );
                        }
                        None => break,
                    }
                }
                if current != accepted
                    && !current.is_empty()
                    && current.chars().count() <= forwarder.notice.max_chars as usize
                {
                    sequence = sequence.saturating_add(1);
                    if deliver_preview_update(
                        &forwarder,
                        &accepted,
                        &current,
                        sequence,
                    )
                    .await
                    {
                        delivered_updates = delivered_updates.saturating_add(1);
                        accepted.clone_from(&current);
                    }
                }
                break;
            },
            item = subscription.next() => match item {
                Some(Ok(envelope)) => {
                    if let Some(text) = live_cumulative_text(&envelope, forwarder.run_id, &mut bodies) {
                        current = text;
                        received_text_updates = received_text_updates.saturating_add(1);
                        if next_update_at.is_none() && current != accepted && !current.is_empty() {
                            next_update_at = Some(next_preview_update_at(
                                last_update_at,
                                tokio::time::Instant::now(),
                            ));
                        }
                    }
                }
                Some(Err(error)) => {
                    tracing::debug!(
                        target: "ironclaw::reborn::run_delivery",
                        run_id = %forwarder.run_id,
                        %error,
                        "progressive preview update unavailable"
                    );
                }
                None => break,
            },
            _ = &mut update_timer, if next_update_at.is_some() => {
                next_update_at = None;
                if current == accepted || current.is_empty() {
                    continue;
                }
                if current.chars().count() > forwarder.notice.max_chars as usize {
                    break;
                }
                let update_started_at = tokio::time::Instant::now();
                sequence = sequence.saturating_add(1);
                if !deliver_preview_update(
                    &forwarder,
                    &accepted,
                    &current,
                    sequence,
                )
                .await
                {
                    break;
                }
                accepted.clone_from(&current);
                delivered_updates = delivered_updates.saturating_add(1);
                last_update_at = Some(update_started_at);
            }
        }
    }
    tracing::debug!(
        target: "ironclaw::reborn::run_delivery",
        run_id = %forwarder.run_id,
        received_text_updates,
        delivered_updates,
        accepted_chars = accepted.chars().count(),
        pending_chars = current.chars().count(),
        "progressive preview forwarder stopped"
    );
}

fn next_preview_update_at(
    last_update_at: Option<tokio::time::Instant>,
    now: tokio::time::Instant,
) -> tokio::time::Instant {
    match last_update_at {
        Some(last_update_at) => std::cmp::max(now, last_update_at + PREVIEW_UPDATE_INTERVAL),
        None => now + FIRST_PREVIEW_UPDATE_DELAY,
    }
}

async fn deliver_preview_update(
    forwarder: &PreviewForwarder,
    accepted_text: &str,
    current_text: &str,
    sequence: u64,
) -> bool {
    let PreviewForwarder {
        services,
        scope,
        actor,
        run_id,
        notice,
        route,
    } = forwarder;
    let authority = ObservedReplyTargetAuthority {
        scope: scope.clone(),
        actor: actor.clone(),
        expected_target: route.reply_target.clone(),
        external_conversation_ref: route.conversation.clone(),
        external_actor_ref: Some(route.actor_ref.clone()),
    };
    let projection_access = AllowNoProjectionAccess;
    let policy = OutboundPolicyService::new(
        services.outbound_store.as_ref(),
        &projection_access,
        &authority,
    );
    let projection_ref = match ProjectionUpdateRef::new(format!("run-preview:{run_id}:{sequence}"))
    {
        Ok(reference) => reference,
        Err(_) => return false,
    };
    let delivery = PrepareCommunicationDeliveryRequest {
        resolution_request: CommunicationDeliveryResolutionRequest {
            scope: scope.clone(),
            actor: actor.clone(),
            modality: CommunicationModality::Text,
            intent: CommunicationDeliveryIntent::RunNotification(RunNotificationContext {
                event_kind: RunNotificationEventKind::ProgressUpdate,
                origin: RunNotificationOrigin::LiveSourceRoute {
                    source_route: SourceRouteContext {
                        reply_target_binding_ref: route.reply_target.clone(),
                    },
                },
            }),
        },
        turn_run_id: Some(*run_id),
        projection_ref,
        attempted_at: Utc::now(),
    };
    let outcome = services
        .coordinator
        .deliver(
            &policy,
            &authority,
            services.project_filesystem.as_ref(),
            CoordinatedDeliveryRequest {
                intent: DeliveryIntent::ProgressivePreview,
                delivery,
                parts: vec![OutboundPart::ProgressivePreview(
                    ProgressivePreviewPart::Update {
                        vendor_message_ref: notice.vendor_message_ref.clone(),
                        accepted_text: accepted_text.to_string(),
                        current_text: current_text.to_string(),
                    },
                )],
                attachments: Vec::new(),
                thread_anchor: None,
                require_direct_message_target: false,
                extension_id: &services.extension_id,
                thread_scope: &route.thread_scope,
            },
        )
        .await;
    matches!(
        outcome,
        Ok(CoordinatedDeliveryOutcome::Delivered { .. }
            | CoordinatedDeliveryOutcome::DeliveredUnconfirmed { .. })
    )
}

fn live_cumulative_text(
    envelope: &ProductOutboundEnvelope,
    run_id: TurnRunId,
    bodies: &mut Vec<(String, String)>,
) -> Option<String> {
    let state = match envelope.payload() {
        ProductOutboundPayload::ProjectionUpdate { state }
        | ProductOutboundPayload::ProjectionSnapshot { state } => state,
        _ => return None,
    };
    let previous: String = bodies.iter().map(|(_, body)| body.as_str()).collect();
    let mut changed = false;
    for item in &state.items {
        if let ProductProjectionItem::Text {
            id,
            run_id: Some(item_run),
            body,
        } = item
            && *item_run == run_id
        {
            if let Some((_, existing)) = bodies.iter_mut().find(|(existing, _)| existing == id) {
                changed |= existing != body;
                existing.clone_from(body);
            } else {
                bodies.push((id.clone(), body.clone()));
                changed = true;
            }
        }
    }
    let current: String = bodies.iter().map(|(_, body)| body.as_str()).collect();
    (changed && current.trim_end() != previous.trim_end()).then_some(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::product_adapter::{AdapterInstallationId, ProductAdapterId};
    use ironclaw_product_contracts::outbound::{ProductOutboundTarget, ProductProjectionState};

    #[test]
    fn preview_cadence_debounces_the_first_update_for_150_milliseconds() {
        let now = tokio::time::Instant::now();

        assert_eq!(
            next_preview_update_at(None, now),
            now + Duration::from_millis(150)
        );
    }

    #[test]
    fn preview_cadence_throttles_follow_up_updates_to_one_second() {
        let first_sent_at = tokio::time::Instant::now();
        let more_text_at = first_sent_at + Duration::from_millis(200);

        assert_eq!(
            next_preview_update_at(Some(first_sent_at), more_text_at),
            first_sent_at + Duration::from_secs(1)
        );
    }

    #[test]
    fn preview_cadence_does_not_delay_text_after_the_throttle_window() {
        let first_sent_at = tokio::time::Instant::now();
        let more_text_at = first_sent_at + Duration::from_secs(2);

        assert_eq!(
            next_preview_update_at(Some(first_sent_at), more_text_at),
            more_text_at
        );
    }

    #[test]
    fn cumulative_text_replaces_phases_in_first_seen_order() {
        let run_id = TurnRunId::new();
        let mut bodies = Vec::new();
        let envelope = |id: &str, body: &str| {
            ProductOutboundEnvelope::new(
                ProductAdapterId::new("slack").expect("adapter"),
                AdapterInstallationId::new("install").expect("installation"),
                ProductOutboundTarget::new(
                    ReplyTargetBindingRef::new("binding").expect("binding"),
                    ExternalConversationRef::new(None, "C1", None, None).expect("conversation"),
                    None,
                ),
                ironclaw_product_contracts::outbound::ProjectionCursor::new("cursor")
                    .expect("cursor"),
                ProductOutboundPayload::ProjectionUpdate {
                    state: ProductProjectionState {
                        thread_id: "thread".to_string(),
                        items: vec![ProductProjectionItem::Text {
                            id: id.to_string(),
                            run_id: Some(run_id),
                            body: body.to_string(),
                        }],
                    },
                },
            )
        };
        assert_eq!(
            live_cumulative_text(&envelope("phase-1", "Hello "), run_id, &mut bodies).as_deref(),
            Some("Hello ")
        );
        assert_eq!(
            live_cumulative_text(&envelope("phase-2", "world"), run_id, &mut bodies).as_deref(),
            Some("Hello world")
        );
        assert_eq!(
            live_cumulative_text(&envelope("phase-1", "Hi "), run_id, &mut bodies).as_deref(),
            Some("Hi world")
        );
        assert_eq!(
            live_cumulative_text(&envelope("phase-2", "world\n"), run_id, &mut bodies),
            None
        );
        assert_eq!(
            live_cumulative_text(&envelope("phase-2", "world\nagain"), run_id, &mut bodies)
                .as_deref(),
            Some("Hi world\nagain")
        );
        assert_eq!(
            live_cumulative_text(&envelope("phase-3", "\n"), run_id, &mut bodies),
            None
        );
        assert_eq!(
            live_cumulative_text(&envelope("phase-3", "\nfinally"), run_id, &mut bodies).as_deref(),
            Some("Hi world\nagain\nfinally")
        );
    }
}
