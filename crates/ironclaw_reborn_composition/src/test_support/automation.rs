//! Test-support constructor for [`crate::RebornAutomationProductFacade`]
//! (W5-WEBUI-API-1 Enabler B.2). Constructor is `pub(crate)` in production;
//! this same-crate wrapper builds the real facade over the harness's shared
//! repository instead of a hand-rolled double duplicating its filter/join logic.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::RootFilesystem;
use ironclaw_outbound::{
    DeliveryTargetCapabilities, OutboundDeliveryTargetEntry, OutboundDeliveryTargetOwner,
    OutboundDeliveryTargetProvider, OutboundDeliveryTargetScope, OutboundDeliveryTargetSummary,
    OutboundError,
};
use ironclaw_product::{AutomationProductFacade, RebornOutboundDeliveryTargetId};
use ironclaw_triggers::{TriggerActiveRunLookup, TriggerRepository};
use ironclaw_turns::{FilesystemTurnStateRowStore, ReplyTargetBindingRef};

use crate::RebornRuntime;
use crate::automation::trigger_poller::SnapshotActiveRunLookup;
use crate::turn_run_snapshot::TurnRunSnapshotSource;

/// Build the production `RebornAutomationProductFacade` over
/// `trigger_repository` plus the harness's own turn-state store, for
/// `RebornServices::with_automation_product_facade`
/// (`ironclaw_product::RebornServices`) test wiring. The turn-state
/// store backs the active-hold projection from the same run state the harness
/// coordinator writes, mirroring production's automation-backing pair (#5886).
#[cfg(feature = "test-support")]
pub fn local_dev_automation_product_facade_for_test<F>(
    trigger_repository: Arc<dyn TriggerRepository>,
    turn_state: Arc<FilesystemTurnStateRowStore<F>>,
) -> Arc<dyn AutomationProductFacade>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    let active_run_lookup = Arc::new(SnapshotActiveRunLookup::new(
        turn_state as Arc<dyn TurnRunSnapshotSource>,
    ));
    Arc::new(
        crate::automation::facade::RebornAutomationProductFacade::new(
            trigger_repository,
            active_run_lookup,
        ),
    )
}

/// Build the raw [`TriggerActiveRunLookup`] the production automation panel
/// wiring uses (`build_local_runtime`'s `trigger_active_run_lookup`), without
/// the `RebornAutomationProductFacade` wrapper. For test harnesses that need
/// to wire the SAME lookup semantics directly into a `builtin.trigger_list`
/// capability registry (`ironclaw_host_runtime::builtin_first_party_handlers_with_trigger_create_hook`)
/// instead of through the WebUI automations facade — see
/// `HostRuntimeCapabilityHarness::install_trigger_active_run_lookup_for_test` (#5886).
#[cfg(feature = "test-support")]
pub fn local_dev_trigger_active_run_lookup_for_test<F>(
    turn_state: Arc<FilesystemTurnStateRowStore<F>>,
) -> Arc<dyn TriggerActiveRunLookup>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    Arc::new(SnapshotActiveRunLookup::new(
        turn_state as Arc<dyn TurnRunSnapshotSource>,
    ))
}

/// Point the production local trigger-create hook at the turn-state store a
/// group integration runtime actually writes. The outbound registry,
/// conversation pairing service, and trigger lifecycle behavior remain the
/// same objects the composed [`RebornServices`] owns.
#[cfg(feature = "test-support")]
pub fn set_local_dev_trigger_source_turn_state_for_test<F>(
    services: &RebornRuntime,
    turn_state: Arc<FilesystemTurnStateRowStore<F>>,
) -> Result<(), String>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    let mut source = services
        .trigger_source_turn_state
        .write()
        .map_err(|error| format!("trigger source turn-state lock unavailable: {error}"))?;
    *source = Arc::clone(&turn_state) as Arc<dyn crate::turn_run_snapshot::TurnRunSnapshotSource>;
    drop(source);
    // Repoint the sibling TurnStateStore-typed slot too so the trigger
    // delivery-target service resolves runs from the same harness store.
    let mut store_slot = services
        .trigger_source_turn_state_store
        .write()
        .map_err(|error| format!("trigger source turn-state store lock unavailable: {error}"))?;
    *store_slot = turn_state as Arc<dyn ironclaw_turns::TurnStateStore>;
    Ok(())
}

struct StaticSourceDeliveryTargetProvider {
    summary: OutboundDeliveryTargetSummary,
    reply_target_binding_ref: ReplyTargetBindingRef,
}

#[async_trait]
impl OutboundDeliveryTargetProvider for StaticSourceDeliveryTargetProvider {
    async fn list_outbound_delivery_targets(
        &self,
        caller: &OutboundDeliveryTargetScope,
    ) -> Result<Vec<OutboundDeliveryTargetEntry>, OutboundError> {
        Ok(vec![OutboundDeliveryTargetEntry {
            summary: self.summary.clone(),
            capabilities: DeliveryTargetCapabilities {
                final_replies: true,
                progress: false,
                gate_prompts: false,
                auth_prompts: false,
                modalities: Vec::new(),
            },
            destination: ironclaw_outbound::RunFinalReplyDestination::External {
                reply_target_binding_ref: self.reply_target_binding_ref.clone(),
            },
            owner: OutboundDeliveryTargetOwner::for_scope(caller),
        }])
    }
}

/// Register one hermetic caller-owned outbound target on the exact mutable
/// registry the production trigger-create hook and triggered-delivery path
/// share. This is test data only; resolution and ownership filtering are the
/// production implementations.
#[cfg(feature = "test-support")]
pub fn register_static_source_delivery_target_for_test(
    services: &RebornRuntime,
    provider_key: impl Into<String>,
    target_id: RebornOutboundDeliveryTargetId,
    reply_target_binding_ref: ReplyTargetBindingRef,
) -> Result<(), String> {
    let registry = services
        .outbound_delivery_target_registry
        .as_ref()
        .ok_or_else(|| "outbound delivery target registry unavailable".to_string())?;
    let summary = OutboundDeliveryTargetSummary::new(
        target_id,
        "test-channel",
        "Source conversation",
        Some("Hermetic source conversation target".to_string()),
    )
    .map_err(|error| format!("invalid test delivery target: {error}"))?;
    registry
        .register_provider(
            provider_key,
            Arc::new(StaticSourceDeliveryTargetProvider {
                summary,
                reply_target_binding_ref,
            }),
        )
        .map(|_| ())
        .map_err(|error| format!("test delivery target registration failed: {error}"))
}
