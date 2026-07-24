//! Test-support constructor for [`crate::RebornAutomationProductService`]
//! (W5-WEBUI-API-1 Enabler B.2). Constructor is `pub(crate)` in production;
//! this same-crate wrapper builds the real service over the harness's shared
//! repository instead of a hand-rolled double duplicating its filter/join logic.

use std::sync::Arc;

use ironclaw_filesystem::RootFilesystem;
use ironclaw_product::AutomationProductService;
use ironclaw_triggers::{TriggerActiveRunLookup, TriggerRepository};
use ironclaw_turns::{TurnStateRowStore, TurnStateStore};

use crate::automation::trigger_poller::SnapshotActiveRunLookup;
use crate::turn_run_snapshot::TurnRunSnapshotSource;

/// Build the production `RebornAutomationProductService` over
/// `trigger_repository` plus the harness's own turn-state store, for
/// `RebornServices::with_automation_product_service`
/// (`ironclaw_product::RebornServices`) test wiring. The turn-state
/// store backs the active-hold projection from the same run state the harness
/// coordinator writes, mirroring production's automation-backing pair (#5886).
#[cfg(feature = "test-support")]
pub fn local_dev_automation_product_service_for_test<F>(
    trigger_repository: Arc<dyn TriggerRepository>,
    turn_state: Arc<TurnStateRowStore<F>>,
) -> Arc<dyn AutomationProductService>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    let active_run_lookup = Arc::new(SnapshotActiveRunLookup::new(
        turn_state as Arc<dyn TurnRunSnapshotSource>,
    ));
    Arc::new(
        crate::automation::service::RebornAutomationProductService::new(
            trigger_repository,
            active_run_lookup,
        ),
    )
}

/// Build the raw [`TriggerActiveRunLookup`] the production automation panel
/// wiring uses (`build_local_runtime`'s `trigger_active_run_lookup`), without
/// the `RebornAutomationProductService` wrapper. For test harnesses that need
/// to wire the SAME lookup semantics directly into a `builtin.trigger_list`
/// capability registry (`ironclaw_host_runtime::builtin_first_party_handlers_with_trigger_create_hook`)
/// instead of through the WebUI automations service — see
/// `HostRuntimeCapabilityHarness::install_trigger_active_run_lookup_for_test` (#5886).
#[cfg(feature = "test-support")]
pub fn local_dev_trigger_active_run_lookup_for_test<F>(
    turn_state: Arc<TurnStateRowStore<F>>,
) -> Arc<dyn TriggerActiveRunLookup>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    Arc::new(SnapshotActiveRunLookup::new(
        turn_state as Arc<dyn TurnRunSnapshotSource>,
    ))
}

/// Repoint the local-dev runtime's trigger-source lookup seams at the harness
/// turn-state store. Integration groups build the capability harness before the
/// group coordinator owns its store, so production's single-store wiring must
/// be late-bound for both active-run listing and trigger delivery inheritance.
#[cfg(feature = "test-support")]
pub fn rebind_local_dev_trigger_source_turn_state_for_test<F>(
    runtime: &crate::RebornRuntime,
    turn_state: Arc<TurnStateRowStore<F>>,
) -> Result<(), String>
where
    F: RootFilesystem + Send + Sync + 'static,
{
    let snapshot_source = Arc::clone(&turn_state) as Arc<dyn TurnRunSnapshotSource>;
    let state_store = turn_state as Arc<dyn TurnStateStore>;
    *runtime
        .trigger_source_turn_state
        .write()
        .map_err(|error| format!("trigger source snapshot lock unavailable: {error}"))? =
        snapshot_source;
    *runtime
        .trigger_source_turn_state_store
        .write()
        .map_err(|error| format!("trigger source turn-state lock unavailable: {error}"))? =
        state_store;
    Ok(())
}
