//! Product-owned routing for trigger-submitted run delivery.
//!
//! Composition supplies the currently assembled channel candidates and the
//! existing current-target resolver. This module owns destination resolution,
//! communication-preference fallback, ambiguity/failure handling, and driver
//! selection so the composition root never decides delivery policy.

use std::sync::Arc;

use chrono::Utc;
use ironclaw_host_api::{InvocationId, ResourceScope};
use ironclaw_outbound::{
    CommunicationPreferenceKey, CommunicationPreferenceRepository, DeliveryDefaultScope,
    TriggerCommunicationContext, TriggerFireSlot, TriggerOriginRef,
    TriggeredRunDeliveryOutcomeKind, TriggeredRunDeliveryRecord, TriggeredRunDeliveryStore,
};
use ironclaw_triggers::TriggerFire;
use ironclaw_turns::{ReplyTargetBindingRef, TurnRunId, TurnScope};

use crate::PreferenceTargetCodec;

use super::{
    CurrentDeliveryTargetResolver, RunDeliveryEventRouter, RunDeliveryServices,
    TriggeredRunDeliveryDriver, TriggeredRunDeliveryRequest, TriggeredRunExternalDeliveryTarget,
};

/// One currently assembled channel lane that can deliver trigger output.
///
/// The vendor codec and delivery services are supplied by the channel host;
/// target selection and failure semantics remain product-owned here.
#[derive(Clone)]
pub struct TriggeredRunDeliveryChannel {
    pub preference_target_codec: Arc<dyn PreferenceTargetCodec>,
    pub services: RunDeliveryServices,
}

/// Generic trigger-result router shared by every channel extension.
pub struct TriggeredRunDeliveryRouter {
    delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
    preferences: Arc<dyn CommunicationPreferenceRepository>,
    current_targets: Arc<dyn CurrentDeliveryTargetResolver>,
    fallback_agent_id: ironclaw_host_api::AgentId,
    event_router: Arc<RunDeliveryEventRouter>,
}

impl TriggeredRunDeliveryRouter {
    pub fn new(
        delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
        preferences: Arc<dyn CommunicationPreferenceRepository>,
        current_targets: Arc<dyn CurrentDeliveryTargetResolver>,
        fallback_agent_id: ironclaw_host_api::AgentId,
        event_router: Arc<RunDeliveryEventRouter>,
    ) -> Self {
        Self {
            delivery_store,
            preferences,
            current_targets,
            fallback_agent_id,
            event_router,
        }
    }

    /// Register the submitted trigger with the channel lane that owns its
    /// exact destination. WebApp is deliberately history-only and therefore
    /// returns without constructing an external delivery driver.
    pub async fn on_trigger_submitted(
        &self,
        fire: TriggerFire,
        run_id: TurnRunId,
        scope: TurnScope,
        channels: Vec<TriggeredRunDeliveryChannel>,
    ) {
        let trigger_context = match triggered_communication_context(&fire) {
            Ok(context) => context,
            Err(reason) => {
                self.warn_and_record_failed(
                    run_id,
                    &reason,
                    "triggered run delivery skipped: cannot build trigger context",
                )
                .await;
                return;
            }
        };
        let resolved_destination = match self.resolve_per_trigger_target(&fire, &scope).await {
            Ok(destination) => destination,
            Err(reason) => {
                self.warn_and_record_failed(
                    run_id,
                    &reason,
                    "triggered run delivery skipped: per-trigger target could not be resolved",
                )
                .await;
                return;
            }
        };
        let Some(external_target) =
            TriggeredRunExternalDeliveryTarget::from_destination(resolved_destination)
        else {
            return;
        };
        let delivery_target = match external_target {
            TriggeredRunExternalDeliveryTarget::UseCommunicationPreference => None,
            TriggeredRunExternalDeliveryTarget::Explicit {
                reply_target_binding_ref,
            } => Some(reply_target_binding_ref),
        };
        let channel = match self
            .route_channel(
                &scope,
                &fire.creator_user_id,
                delivery_target.as_ref(),
                channels,
            )
            .await
        {
            Ok(channel) => channel,
            Err(reason) => {
                self.warn_and_record_failed(
                    run_id,
                    &reason,
                    "triggered run delivery skipped: no channel extension owns the delivery",
                )
                .await;
                return;
            }
        };
        let driver = TriggeredRunDeliveryDriver::with_event_router(
            channel.services,
            Arc::clone(&self.delivery_store),
            Arc::clone(&self.current_targets),
            self.fallback_agent_id.clone(),
            Arc::clone(&self.event_router),
        );
        driver
            .on_trigger_submitted(TriggeredRunDeliveryRequest {
                run_id,
                scope,
                creator_user_id: fire.creator_user_id.clone(),
                project_scoped: fire.project_id.is_some(),
                prompt: fire.prompt.clone(),
                delivery_target,
                trigger_context,
            })
            .await;
    }

    async fn resolve_per_trigger_target(
        &self,
        fire: &TriggerFire,
        scope: &TurnScope,
    ) -> Result<Option<ironclaw_outbound::RunFinalReplyDestination>, String> {
        let Some(target) = fire.delivery_target.as_ref() else {
            return Ok(None);
        };
        let caller_scope = ResourceScope {
            tenant_id: scope.tenant_id.clone(),
            user_id: fire.creator_user_id.clone(),
            agent_id: fire.agent_id.clone(),
            project_id: fire.project_id.clone(),
            mission_id: None,
            thread_id: Some(scope.thread_id.clone()),
            invocation_id: InvocationId::new(),
        };
        self.current_targets
            .resolve_current_destination(&caller_scope, target)
            .await
            .map_err(|error| format!("per-trigger delivery target lookup failed: {error}"))?
            .ok_or_else(|| {
                "per-trigger delivery target is no longer available to its creator".to_string()
            })
            .map(Some)
    }

    async fn route_channel(
        &self,
        scope: &TurnScope,
        creator: &ironclaw_host_api::UserId,
        per_trigger_target: Option<&ReplyTargetBindingRef>,
        channels: Vec<TriggeredRunDeliveryChannel>,
    ) -> Result<TriggeredRunDeliveryChannel, String> {
        if channels.is_empty() {
            return Err("no active channel extension registered a preference codec".to_string());
        }
        let target = match per_trigger_target {
            Some(target) => Some(target.clone()),
            None => self.stored_preference_target(scope, creator).await?,
        };
        if let Some(target) = target {
            return channels
                .into_iter()
                .find(|channel| {
                    channel
                        .preference_target_codec
                        .conversation_for_target(&target)
                        .is_some()
                })
                .ok_or_else(|| {
                    format!(
                        "no registered channel codec decodes the stored preference target `{}`",
                        target.as_str()
                    )
                });
        }
        let mut channels = channels.into_iter();
        if let (Some(channel), None) = (channels.next(), channels.next()) {
            return Ok(channel);
        }
        Err(
            "no stored preference target and several channel extensions are active; \
             delivery routing is ambiguous"
                .to_string(),
        )
    }

    async fn stored_preference_target(
        &self,
        scope: &TurnScope,
        creator: &ironclaw_host_api::UserId,
    ) -> Result<Option<ReplyTargetBindingRef>, String> {
        let key = CommunicationPreferenceKey {
            scope: DeliveryDefaultScope::personal(scope.tenant_id.clone(), creator.clone()),
        };
        let record = self
            .preferences
            .load_communication_preference(key)
            .await
            .map_err(|error| format!("communication preference read failed: {error}"))?;
        Ok(record.and_then(|versioned| {
            let record = versioned.record;
            record
                .final_reply_target
                .or(record.approval_prompt_target)
                .or(record.auth_prompt_target)
                .or(record.progress_target)
        }))
    }

    async fn warn_and_record_failed(&self, run_id: TurnRunId, reason: &str, message: &str) {
        tracing::warn!(
            target = "ironclaw::product_workflow::triggered_delivery",
            %run_id,
            %reason,
            "{message}"
        );
        let record = TriggeredRunDeliveryRecord {
            run_id,
            outcome: TriggeredRunDeliveryOutcomeKind::Failed,
            recorded_at: Utc::now(),
        };
        if let Err(error) = self
            .delivery_store
            .record_triggered_run_delivery(record)
            .await
        {
            tracing::warn!(
                target = "ironclaw::product_workflow::triggered_delivery",
                %run_id,
                %error,
                "failed to record triggered run delivery outcome (best-effort)"
            );
        }
    }
}

/// Build the generic communication context carried by every scheduled fire.
fn triggered_communication_context(
    fire: &TriggerFire,
) -> Result<TriggerCommunicationContext, String> {
    let trigger_origin_ref = TriggerOriginRef::new(fire.identity.trigger_id().to_string())
        .map_err(|error| format!("invalid trigger origin ref: {error}"))?;
    let fire_slot = TriggerFireSlot::new(fire.identity.fire_slot().to_rfc3339())
        .map_err(|error| format!("invalid fire slot: {error}"))?;
    Ok(TriggerCommunicationContext {
        trigger_origin_ref,
        trigger_source_kind: ironclaw_outbound::TriggerSourceKind::Schedule,
        fire_slot,
    })
}
