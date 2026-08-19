//! Test-support constructor for
//! [`ironclaw_assistant::RebornAutomationProductService`] (W5-WEBUI-API-1
//! Enabler B.2). This wrapper builds the real service over the harness's shared
//! repository instead of a hand-rolled double duplicating its filter/join logic.

use std::sync::Arc;

use ironclaw_assistant::AutomationProductService;
use ironclaw_processes::ProcessLifecycleLookupSource;
use ironclaw_triggers::{TriggerActiveRunLookup, TriggerRepository};
use ironclaw_turns::TurnError;

use crate::automation::trigger_poller::ProcessActiveRunLookup;

/// Build the production `RebornAutomationProductService` over
/// `trigger_repository` plus the harness's own process lifecycle source, for
/// `RebornServices::with_automation_product_service`
/// (`ironclaw_assistant::RebornServices`) test wiring. The process source backs
/// the active-hold projection from the same journal the harness coordinator
/// writes, mirroring production's automation-backing pair (#5886).
#[cfg(feature = "test-support")]
pub fn standalone_automation_product_service_for_test(
    trigger_repository: Arc<dyn TriggerRepository>,
    processes: Arc<dyn ProcessLifecycleLookupSource<Error = TurnError>>,
) -> Arc<dyn AutomationProductService> {
    let active_run_lookup = Arc::new(ProcessActiveRunLookup::new(processes));
    Arc::new(ironclaw_assistant::RebornAutomationProductService::new(
        trigger_repository,
        active_run_lookup,
    ))
}

/// Build the raw [`TriggerActiveRunLookup`] the production automation panel
/// wiring uses (`build_local_runtime`'s `trigger_active_run_lookup`), without
/// the `RebornAutomationProductService` wrapper. For test harnesses that need
/// to wire the SAME lookup semantics directly into a `builtin.trigger_list`
/// capability registry (`ironclaw_host_runtime::builtin_first_party_handlers_with_trigger_create_hook`)
/// instead of through the WebUI automations service — see
/// `HostRuntimeCapabilityHarness::install_trigger_active_run_lookup_for_test` (#5886).
#[cfg(feature = "test-support")]
pub fn standalone_trigger_active_run_lookup_for_test(
    processes: Arc<dyn ProcessLifecycleLookupSource<Error = TurnError>>,
) -> Arc<dyn TriggerActiveRunLookup> {
    Arc::new(ProcessActiveRunLookup::new(processes))
}

/// Repoint the standalone runtime's trigger-source lifecycle lookup at the
/// harness process runtime. Integration groups build the capability harness
/// before the group coordinator owns its runtime, so production's
/// single-system wiring must be late-bound for active-run listing.
#[cfg(feature = "test-support")]
pub fn rebind_standalone_trigger_source_turn_state_for_test(
    runtime: &crate::RebornRuntime,
    lifecycle_source: Arc<dyn ProcessLifecycleLookupSource<Error = TurnError>>,
) -> Result<(), String> {
    *runtime
        .trigger_process_lifecycle_source
        .write()
        .map_err(|error| format!("trigger source lifecycle lock unavailable: {error}"))? =
        lifecycle_source;
    Ok(())
}
