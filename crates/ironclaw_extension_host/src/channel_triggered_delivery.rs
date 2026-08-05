//! Generic triggered-run delivery over the channel host assembly
//! (extension-runtime §5.4, P6 c-rest).
//!
//! One composition-owned [`PostSubmitDeliveryHook`] serves every channel
//! extension: on a settled trigger fire it resolves the fire's optional
//! creator-owned target id, otherwise reads the creator's personal
//! communication preference, routes that reply-target binding ref to the
//! extension whose registered [`PreferenceTargetCodec`] decodes it, and drives
//! that extension's generic [`TriggeredRunDelivery`] driver. The single poller
//! hook slot stays — multiplexing happens inside this hook, by extension id.
//!
//! The drivers themselves are built by the product-side channel workflow
//! factory and supplied by composition (§12.11 D-A): this module owns the
//! *routing* policy — which extension a fire belongs to, and the fail-closed
//! rules below — and names no product type to do it.
//!
//! Fail-closed routing: with no stored preference the fire routes to the
//! only active codec-bearing channel extension when exactly one exists (its
//! driver then records the no-default outcome through the normal path);
//! with zero or several candidates the outcome is recorded as `Failed`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_contracts::preference_target::PreferenceTargetCodec;
use ironclaw_outbound::{
    CommunicationPreferenceKey, CommunicationPreferenceRepository, DeliveryDefaultScope,
    OutboundDeliveryTargetId, OutboundDeliveryTargetScope, TriggeredRunDelivery,
    TriggeredRunDeliveryOutcomeKind, TriggeredRunDeliveryRecord, TriggeredRunDeliveryRequest,
    TriggeredRunDeliveryStore,
};
use ironclaw_triggers::TriggerFire;
use ironclaw_turns::{ReplyTargetBindingRef, TurnRunId, TurnScope};

use crate::channel_host::GenericChannelHostAssembly;
use ironclaw_outbound::{MutableOutboundDeliveryTargetRegistry, OutboundDeliveryTargetProvider};

/// Hook invoked by the trigger poller after a successful fire submission has
/// been durably settled in trigger storage.
#[async_trait::async_trait]
pub trait PostSubmitDeliveryHook: Send + Sync {
    /// Called with the original trigger fire, the submitted run id, and the
    /// turn scope the run was submitted under.
    async fn on_trigger_submitted(&self, fire: TriggerFire, run_id: TurnRunId, scope: TurnScope);
}

/// The generic post-submit delivery hook: routes each settled trigger fire
/// to the owning extension's triggered-delivery driver.
pub struct GenericTriggeredRunDeliveryHook {
    assembly: Arc<GenericChannelHostAssembly>,
    delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
    preferences: Arc<dyn CommunicationPreferenceRepository>,
    delivery_targets: Arc<MutableOutboundDeliveryTargetRegistry>,
    /// One driver per channel extension, keyed by extension id, built by the
    /// product-side workflow factory (§12.11 D-A) and supplied by composition
    /// from the same channel-extension bindings that register the codecs this
    /// hook routes across. Routing stays here; construction does not.
    drivers: HashMap<String, Arc<dyn TriggeredRunDelivery>>,
}

impl GenericTriggeredRunDeliveryHook {
    pub fn new(
        assembly: Arc<GenericChannelHostAssembly>,
        delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
        preferences: Arc<dyn CommunicationPreferenceRepository>,
        delivery_targets: Arc<MutableOutboundDeliveryTargetRegistry>,
        drivers: impl IntoIterator<Item = (String, Arc<dyn TriggeredRunDelivery>)>,
    ) -> Self {
        Self {
            assembly,
            delivery_store,
            preferences,
            delivery_targets,
            drivers: drivers.into_iter().collect(),
        }
    }

    /// Pick the extension that owns this fire's delivery: the one whose
    /// codec decodes the creator's stored preference target; falling back
    /// to the only codec-bearing active channel extension when no target is
    /// stored (single-channel deployments keep the retired lane's behavior
    /// of recording the no-default outcome through the driver).
    async fn route_extension(
        &self,
        scope: &TurnScope,
        creator: &ironclaw_host_api::ids::UserId,
        per_trigger_target: Option<&ReplyTargetBindingRef>,
    ) -> Result<(String, Arc<dyn PreferenceTargetCodec>), String> {
        let codecs = self.assembly.active_preference_codecs();
        if codecs.is_empty() {
            return Err("no active channel extension registered a preference codec".to_string());
        }
        let target = match per_trigger_target {
            Some(target) => Some(target.clone()),
            None => self.stored_preference_target(scope, creator).await?,
        };
        if let Some(target) = target {
            for (extension_id, codec) in &codecs {
                if codec.conversation_for_target(&target).is_some() {
                    return Ok((extension_id.clone(), Arc::clone(codec)));
                }
            }
            return Err(format!(
                "no registered channel codec decodes the stored preference target `{}`",
                target.as_str()
            ));
        }
        let mut codecs = codecs.into_iter();
        if let (Some((extension_id, codec)), None) = (codecs.next(), codecs.next()) {
            return Ok((extension_id, codec));
        }
        Err(
            "no stored preference target and several channel extensions are active; \
             delivery routing is ambiguous"
                .to_string(),
        )
    }

    /// Resolve the fire's durable opaque target id at fire time through the
    /// same creator-scoped registry used at trigger creation. This both turns
    /// the public id into a transport binding and revalidates ownership and
    /// current availability after any intervening channel disconnect/remove.
    async fn resolve_per_trigger_target(
        &self,
        fire: &TriggerFire,
        scope: &TurnScope,
    ) -> Result<Option<ReplyTargetBindingRef>, String> {
        let Some(target) = fire.delivery_target.as_ref() else {
            return Ok(None);
        };
        let target_id = OutboundDeliveryTargetId::new(target.as_str())
            .map_err(|error| format!("invalid per-trigger delivery target id: {error}"))?;
        let caller =
            OutboundDeliveryTargetScope::new(scope.tenant_id.clone(), fire.creator_user_id.clone());
        let entry = self
            .delivery_targets
            .resolve_outbound_delivery_target(&caller, &target_id)
            .await
            .map_err(|error| format!("per-trigger delivery target lookup failed: {error}"))?
            .ok_or_else(|| {
                "per-trigger delivery target is no longer available to its creator".to_string()
            })?;
        match entry.destination {
            ironclaw_outbound::RunFinalReplyDestination::External {
                reply_target_binding_ref,
            } => Ok(Some(reply_target_binding_ref)),
            ironclaw_outbound::RunFinalReplyDestination::WebApp => Ok(None),
        }
    }

    /// The creator's first configured personal preference target, in
    /// delivery-role order.
    async fn stored_preference_target(
        &self,
        scope: &TurnScope,
        creator: &ironclaw_host_api::ids::UserId,
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

    fn driver_for_extension(
        &self,
        extension_id: &str,
    ) -> Result<Arc<dyn TriggeredRunDelivery>, String> {
        self.drivers
            .get(extension_id)
            .map(Arc::clone)
            .ok_or_else(|| {
                "composed runtime has no delivery coordinator; triggered delivery unavailable"
                    .to_string()
            })
    }

    async fn record_failed(&self, run_id: TurnRunId) {
        let record = TriggeredRunDeliveryRecord {
            run_id,
            outcome: TriggeredRunDeliveryOutcomeKind::Failed,
            recorded_at: chrono::Utc::now(),
        };
        if let Err(error) = self
            .delivery_store
            .record_triggered_run_delivery(record)
            .await
        {
            tracing::warn!(
                target = "ironclaw::reborn::channel_triggered_delivery",
                %run_id,
                %error,
                "failed to record triggered run delivery outcome (best-effort)"
            );
        }
    }
}

#[async_trait]
impl PostSubmitDeliveryHook for GenericTriggeredRunDeliveryHook {
    async fn on_trigger_submitted(&self, fire: TriggerFire, run_id: TurnRunId, scope: TurnScope) {
        let trigger_context = match triggered_communication_context(&fire) {
            Ok(context) => context,
            Err(reason) => {
                tracing::warn!(
                    target = "ironclaw::reborn::channel_triggered_delivery",
                    %run_id,
                    %reason,
                    "triggered run delivery skipped: cannot build trigger context"
                );
                self.record_failed(run_id).await;
                return;
            }
        };
        let delivery_target = match self.resolve_per_trigger_target(&fire, &scope).await {
            Ok(target) => target,
            Err(reason) => {
                tracing::warn!(
                    target = "ironclaw::reborn::channel_triggered_delivery",
                    %run_id,
                    %reason,
                    "triggered run delivery skipped: per-trigger target could not be resolved"
                );
                self.record_failed(run_id).await;
                return;
            }
        };
        let (extension_id, _codec) = match self
            .route_extension(&scope, &fire.creator_user_id, delivery_target.as_ref())
            .await
        {
            Ok(routed) => routed,
            Err(reason) => {
                tracing::warn!(
                    target = "ironclaw::reborn::channel_triggered_delivery",
                    %run_id,
                    %reason,
                    "triggered run delivery skipped: no channel extension owns the delivery"
                );
                self.record_failed(run_id).await;
                return;
            }
        };
        let driver = match self.driver_for_extension(&extension_id) {
            Ok(driver) => driver,
            Err(reason) => {
                tracing::warn!(
                    target = "ironclaw::reborn::channel_triggered_delivery",
                    %run_id,
                    extension_id,
                    %reason,
                    "triggered run delivery skipped: delivery driver unavailable"
                );
                self.record_failed(run_id).await;
                return;
            }
        };
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
}

/// Build a [`ironclaw_outbound::TriggerCommunicationContext`] from the
/// fire's identity — the generic translation the retired per-vendor hooks
/// each carried.
pub fn triggered_communication_context(
    fire: &TriggerFire,
) -> Result<ironclaw_outbound::TriggerCommunicationContext, String> {
    let trigger_origin_ref =
        ironclaw_outbound::TriggerOriginRef::new(fire.identity.trigger_id().to_string())
            .map_err(|error| format!("invalid trigger origin ref: {error}"))?;
    let fire_slot = ironclaw_outbound::TriggerFireSlot::new(fire.identity.fire_slot().to_rfc3339())
        .map_err(|error| format!("invalid fire slot: {error}"))?;
    Ok(ironclaw_outbound::TriggerCommunicationContext {
        trigger_origin_ref,
        trigger_source_kind: ironclaw_outbound::TriggerSourceKind::Schedule,
        fire_slot,
    })
}
