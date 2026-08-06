//! Generic background-run notification over the channel host assembly
//! (extension-runtime §5.4, P6 c-rest).
//!
//! One composition-owned [`PostSubmitDeliveryHook`] serves every channel
//! extension. On a settled trigger fire it hands the run to the single
//! background-run notifier, which resolves the creator's stored
//! **notification channels** at fire time and fans gate/auth/failure notices
//! out over every one of them.
//!
//! The hook does no routing. There is no per-fire destination to pick any
//! more: a fire's result is never pushed (spec §8), and a notification goes to
//! every configured channel rather than to one chosen extension. The notifier
//! itself is built by the product-side channel workflow factory and supplied
//! by composition (§12.11 D-A): this module names no product type — it owns
//! only the hand-off and the fail-closed recording below, plus the live
//! codec view every notification target decodes through.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_extension_contracts::preference_target::{
    ActivePreferenceTargetCodecs, PreferenceTargetCodec,
};
use ironclaw_outbound::{
    TriggeredRunDelivery, TriggeredRunDeliveryOutcomeKind, TriggeredRunDeliveryRecord,
    TriggeredRunDeliveryRequest, TriggeredRunDeliveryStore,
};
use ironclaw_triggers::TriggerFire;
use ironclaw_turns::{TurnRunId, TurnScope};

use crate::channel_host::GenericChannelHostAssembly;

/// Attribution bucket for the notifier's own run-delivery services. The
/// notifier is NOT bound to one extension — it chooses the delivering
/// extension per notification target from that target's catalog entry — so
/// this id never selects a channel; it only names the notifier's notice
/// ledger scope. Composition uses it to build the notifier's services.
pub const BACKGROUND_RUN_NOTIFIER_ID: &str = "background-run-notifier";

/// Hook invoked by the trigger poller after a successful fire submission has
/// been durably settled in trigger storage.
#[async_trait::async_trait]
pub trait PostSubmitDeliveryHook: Send + Sync {
    /// Called with the original trigger fire, the submitted run id, and the
    /// turn scope the run was submitted under.
    async fn on_trigger_submitted(&self, fire: TriggerFire, run_id: TurnRunId, scope: TurnScope);
}

/// The generic post-submit delivery hook: hands each settled trigger fire to
/// the background-run notifier.
pub struct GenericTriggeredRunDeliveryHook {
    /// The single background-run notifier, built by the product-side workflow
    /// factory and supplied by composition (§12.11 D-A). `None` when the
    /// composed runtime has no delivery coordinator — every fire then records
    /// a `Failed` outcome instead of notifying (fail closed).
    notifier: Option<Arc<dyn TriggeredRunDelivery>>,
    delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
}

impl GenericTriggeredRunDeliveryHook {
    pub fn new(
        notifier: Option<Arc<dyn TriggeredRunDelivery>>,
        delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
    ) -> Self {
        Self {
            notifier,
            delivery_store,
        }
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
                target: "ironclaw::reborn::channel_triggered_delivery",
                %run_id,
                %error,
                "failed to record background run notification outcome (best-effort)"
            );
        }
    }
}

#[async_trait]
impl PostSubmitDeliveryHook for GenericTriggeredRunDeliveryHook {
    async fn on_trigger_submitted(&self, fire: TriggerFire, run_id: TurnRunId, scope: TurnScope) {
        let Some(notifier) = self.notifier.as_ref() else {
            tracing::warn!(
                target: "ironclaw::reborn::channel_triggered_delivery",
                %run_id,
                "background run notification skipped: composed runtime has no delivery \
                 coordinator"
            );
            self.record_failed(run_id).await;
            return;
        };
        // A fire carries no destination at all any more (spec §8): the stored
        // per-trigger route is gone from `TriggerFire`, and composition's boot
        // migration rewrites any surviving column value into the routine's own
        // prompt. Delivery happens only if the fire calls
        // `builtin.outbound_deliver` itself.
        notifier
            .on_trigger_submitted(TriggeredRunDeliveryRequest {
                run_id,
                scope,
                creator_user_id: fire.creator_user_id.clone(),
                project_scoped: fire.project_id.is_some(),
                prompt: fire.prompt.clone(),
            })
            .await;
    }
}

/// Live codec view over the channel host assembly's ACTIVE snapshot. Each
/// call re-reads the snapshot, so activations and deactivations are visible to
/// the long-lived notifier without rebuilding it. Capturing a snapshot
/// instead would freeze the channel set at first use and leave every
/// later-activated channel's gate prompts undeliverable until restart.
pub struct AssemblyPreferenceTargetCodecs {
    assembly: Arc<GenericChannelHostAssembly>,
}

impl AssemblyPreferenceTargetCodecs {
    pub fn new(assembly: Arc<GenericChannelHostAssembly>) -> Self {
        Self { assembly }
    }
}

impl ActivePreferenceTargetCodecs for AssemblyPreferenceTargetCodecs {
    fn active_preference_target_codecs(&self) -> Vec<Arc<dyn PreferenceTargetCodec>> {
        self.assembly
            .active_preference_codecs()
            .into_iter()
            .map(|(_, codec)| codec)
            .collect()
    }
}
